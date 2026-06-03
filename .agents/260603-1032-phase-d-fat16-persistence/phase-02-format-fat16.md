# Phase 2: Format FAT16 Region in gen_disk.ps1

## Context Links
- `tools/mkfat32.py` — proven FAT16 BPB writer (DRY source; despite the name it emits FAT16)
- `tools/write-cell-table.py:73-98` — extends image; cell table header at LBA 82000
- `gen_disk.ps1:106-131` — blank image (81920 sectors) then cell table append
- `kernel/src/loader/disk_layout.rs:22` — `CELL_TABLE_BASE_LBA = 82_000`

## Overview
- **Priority:** P1 (parallel with Phase 1; blocks Phase 5)
- **Status:** pending
- **Effort:** 2h
- Lay an empty FAT16 filesystem on LBA 0 of `disk_v3.img`, occupying ≤ 81920
  sectors, BEFORE the cell table is appended at LBA 82000.

## Key Insights (verified)
- Disk image is created blank at **81920 sectors** (`gen_disk.ps1:108`), NOT
  82000 as the research stated. `write-cell-table.py` then pads/extends the image
  so the header lands at exactly `82000 * 512`. **FAT16 must declare ≤ 81920
  total sectors** to avoid claiming the pad zone (81920–81999) or the cell table.
- `mkfat32.py` already writes a spec-correct FAT16 BPB (`\xEB\x3C\x90`, BPB fields,
  `FAT16   ` type label, `0x55AA` at 510). Reuse this logic; DON'T reinvent it.
- mkfat32.py *creates a fresh file* and zero-fills to `sector_count`. For Phase D
  we must write IN-PLACE into the already-created `disk_v3.img` at offset 0, and
  must NOT seek-to-end / extend (that would clobber nothing yet, but ordering
  matters — format runs BEFORE the cell-table step that extends to 82000+).
- The FAT16 region holds NO files at format time — `/data/` files are created at
  runtime via the VFS write path. Only BPB + 2 FATs + empty root dir are written.

## Requirements
- **Functional:** After format, offsets 0..(data_start) of `disk_v3.img` contain a
  valid empty FAT16 volume of 81920 sectors; `fatfs::FileSystem::new()` mounts it.
- **Non-functional:** Format must be idempotent and must not touch LBA ≥ 81920.
  The script must run before the cell-table append in `gen_disk.ps1`.

## Architecture
```
gen_disk.ps1
  3b. create blank 81920-sector disk_v3.img
  3c. python tools/mkfat16.py disk_v3.img 81920   ◀── NEW (format LBA 0)
  4.  python tools/write-cell-table.py disk_v3.img ... ◀── extends to LBA 82000+
```
FAT16 geometry at 81920 sectors, 8 sectors/cluster (4096 B):
- reserved=1, FATs=2, root_entries=512 (32 sectors)
- data_clusters ≈ (81920 − 1 − 2·FATsz − 32) / 8 ≈ ~10,200 → inside FAT16 window
  (4085..65524). Confirms FAT16 (not FAT12, not FAT32).

## Related Code Files
**Create:** `tools/mkfat16.py`
**Modify:** `gen_disk.ps1` (insert step 3c after 3b)

## Implementation Steps

### 1. Create `tools/mkfat16.py`
Adapt `mkfat32.py` to: (a) write in-place to an existing file at offset 0,
(b) take `total_sectors` as an argument, (c) use 8 sectors/cluster, (d) write an
EMPTY root dir (no files). Skeleton:
```python
#!/usr/bin/env python3
"""mkfat16.py — write an EMPTY FAT16 filesystem to LBA 0 of an existing image.

Usage: python mkfat16.py <image_path> <total_sectors>

Unlike mkfat32.py (which creates a standalone image), this writes IN-PLACE into
an already-allocated disk image WITHOUT extending it, so the cell bootstrap table
appended later at LBA 82000 is never disturbed. total_sectors must be <= the
sectors available before CELL_TABLE_BASE_LBA (81920 for disk_v3.img).
"""
import struct, sys

SECTOR_SIZE   = 512
SEC_PER_CLUS  = 8          # 4096-byte clusters
RESERVED      = 1
FATS          = 2
ROOT_ENTRIES  = 512
CELL_TABLE_BASE_LBA = 82_000   # must match disk_layout.rs

def fat16_geometry(total_sectors):
    root_dir_sectors = (ROOT_ENTRIES * 32 + SECTOR_SIZE - 1) // SECTOR_SIZE  # 32
    # Iterate to converge FAT size (FAT entries are 2 bytes).
    fat_sectors = 1
    for _ in range(8):
        data_sectors = total_sectors - RESERVED - FATS * fat_sectors - root_dir_sectors
        clusters = data_sectors // SEC_PER_CLUS
        fat_bytes = (clusters + 2) * 2
        new_fat = (fat_bytes + SECTOR_SIZE - 1) // SECTOR_SIZE
        if new_fat == fat_sectors:
            break
        fat_sectors = new_fat
    data_start = RESERVED + FATS * fat_sectors + root_dir_sectors
    clusters = (total_sectors - data_start) // SEC_PER_CLUS
    if not (4085 <= clusters <= 65524):
        raise SystemExit(f"cluster count {clusters} outside FAT16 window")
    return fat_sectors, root_dir_sectors, data_start, clusters

def main():
    img, total = sys.argv[1], int(sys.argv[2])
    if (total + 1 + 32) > CELL_TABLE_BASE_LBA:   # leave headroom before table
        raise SystemExit(f"total_sectors {total} too close to cell table LBA")
    fat_sectors, root_dir_sectors, data_start, clusters = fat16_geometry(total)

    # Build boot sector (BPB) — mirror mkfat32.py exactly, but SEC_PER_CLUS=8.
    boot = bytearray(SECTOR_SIZE)
    boot[0:3]   = b'\xEB\x3C\x90'
    boot[3:11]  = b'MSWIN4.1'
    struct.pack_into('<H', boot, 11, SECTOR_SIZE)
    boot[13]    = SEC_PER_CLUS
    struct.pack_into('<H', boot, 14, RESERVED)
    boot[16]    = FATS
    struct.pack_into('<H', boot, 17, ROOT_ENTRIES)
    struct.pack_into('<H', boot, 19, total if total < 0x10000 else 0)
    boot[21]    = 0xF8
    struct.pack_into('<H', boot, 22, fat_sectors)
    struct.pack_into('<H', boot, 24, 63)    # sectors/track
    struct.pack_into('<H', boot, 26, 255)   # heads
    struct.pack_into('<I', boot, 28, 0)     # hidden
    struct.pack_into('<I', boot, 32, total if total >= 0x10000 else 0)
    boot[36]    = 0x80                       # drive num
    boot[38]    = 0x29                       # boot sig
    struct.pack_into('<I', boot, 39, 0x56494F44)  # vol id
    boot[43:54] = b'ViOS DATA  '            # 11-byte label
    boot[54:62] = b'FAT16   '               # 8-byte fs type
    boot[510:512] = b'\x55\xAA'

    # FAT[0]=media+EOC, FAT[1]=EOC, rest zero (empty volume).
    fat = bytearray(fat_sectors * SECTOR_SIZE)
    struct.pack_into('<H', fat, 0, 0xFFF8)
    struct.pack_into('<H', fat, 2, 0xFFFF)

    # Write IN-PLACE (r+b) — never seek past data_start; do NOT extend file.
    with open(img, 'r+b') as f:
        f.seek(0);                       f.write(boot)
        f.seek(RESERVED * SECTOR_SIZE);  f.write(fat)
        f.seek((RESERVED + fat_sectors) * SECTOR_SIZE); f.write(fat)
        # Zero the root-dir region so no stale entries are interpreted.
        f.seek(data_start * SECTOR_SIZE - root_dir_sectors * SECTOR_SIZE)
        f.write(b'\x00' * (root_dir_sectors * SECTOR_SIZE))
    print(f"[mkfat16] {img}: {total} sectors, {clusters} clusters, "
          f"FATsz={fat_sectors}, data_start=LBA {data_start}")

if __name__ == '__main__':
    main()
```
NOTE on root-dir zeroing: root dir region sits at
`RESERVED + FATS*fat_sectors .. data_start`. The seek expression above equals
`(RESERVED + FATS*fat_sectors) * SECTOR_SIZE` — verify when implementing
(`data_start - root_dir_sectors == RESERVED + FATS*fat_sectors`).

### 2. Insert format step in `gen_disk.ps1` (after line 111, before line 113)
```powershell
# 3c. Format an empty FAT16 filesystem on LBA 0-81919 (before cell table at 82000).
Write-Host "Formatting FAT16 partition (LBA 0-81919)..."
python "$tools_dir\mkfat16.py" "disk_v3.img" 81920 2>&1
```

### 3. Verify the BPB
```powershell
# offset 510 must be 55 AA; offset 54 must read "FAT16   "
$bytes = [System.IO.File]::ReadAllBytes("disk_v3.img")[0..63]
```
Or via Python: assert `bytes[510:512] == b'\x55\xAA'` and `bytes[54:62] == b'FAT16   '`.

## Todo List
- [ ] Create `tools/mkfat16.py` (in-place, no extend, SEC_PER_CLUS=8)
- [ ] Verify cluster count lands in 4085..65524
- [ ] Insert step 3c in `gen_disk.ps1`
- [ ] Run `./gen_disk.ps1`; confirm cell table still readable (boot test in Phase 5)
- [ ] Verify `0x55AA` @ 510 and `FAT16   ` @ 54 after format

## Success Criteria
- `mkfat16.py` exits 0 and prints cluster count in FAT16 window.
- After `gen_disk.ps1`: BPB magic and fs-type label correct; existing
  `boots_to_shell_prompt` / `fat_filesystem_mounts` tests still pass (cell table
  at LBA 82000 intact — proves format didn't extend past 81920).

## Risk Assessment
| Risk | L | I | Mitigation |
|------|---|---|-----------|
| Format extends file past 81920 → clobbers nothing but breaks ordering assumption | Low | Med | Use `r+b`, never seek ≥ data_start*512 except writes within region |
| Cluster count drifts to FAT12 (<4085) or FAT32 (>65524) | Low | High | Explicit guard raises SystemExit |
| gen_disk.ps1 step order wrong (format after table) | Med | High | Insert 3c strictly between 3b and 4; document |
| Root-dir region not zeroed → stale entries | Low | Med | Explicit zero-fill of root dir; fresh blank image is already zero |

## Security Considerations
- Build-time only; no runtime trust boundary. FAT16 region cannot reach LBA 82000.

## Next Steps
Phase 5 boots the formatted disk and exercises write+read through the VFS.

## Evidence

**Tool Creation & Integration:**
- `tools/mkfat16.py` — created with in-place formatting, FAT16 geometry validation, root-dir zeroing
- `gen_disk.ps1` — step 3c added after blank image creation (line 113), before cell-table append

**Output & Verification:**
- `gen_disk.ps1` execution prints: `[mkfat16] disk_v3.img: 81920 sectors, 10225 clusters, FATsz=40, data_start=LBA 113`
- Cluster count 10225 is within FAT16 window (4085–65524) ✓
- BPB magic at offset 510: `0x55AA` ✓
- FS-type label at offset 54–61: `FAT16   ` ✓
- Cell table at LBA 82000 remains intact (existing `fat_filesystem_mounts` test still passes)

**Calculation Verification:** `data_start - root_dir_sectors == RESERVED + FATS*fat_sectors` holds (113 − 32 == 1 + 2*40) ✓

## Unresolved Questions
- None blocking. Confirm at implementation that
  `data_start - root_dir_sectors == RESERVED + FATS*fat_sectors` (it does by
  definition) so the root-dir zero-fill seek is correct.
