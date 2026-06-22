"""mkfat32_inplace.py — Write an empty FAT32 filesystem to LBA 0 of an existing image.

Usage:
    python mkfat32_inplace.py <image_path> <total_sectors>

Writes IN-PLACE into an already-allocated disk image WITHOUT extending it. The caller
is responsible for ensuring the image is large enough.

FAT32 minimum: 65,525 data clusters. At 8 sectors/cluster (4096-byte clusters):
  minimum total_sectors ≥ 65,525 × 8 + overhead ≈ 524,200 sectors (~256 MB).

This script is idempotent: re-running it overwrites the BPB, FSInfo, and FATs
but leaves the data region untouched.
"""

import struct
import sys

SECTOR_SIZE       = 512
# Preferred cluster sizes, largest first. The formatter picks the first one
# that yields ≥ 65,525 data clusters (the FAT32 minimum) for the given volume
# size — e.g. the 524,288-sector P1 partition needs 4 sec/clus (2 KiB clusters)
# because at 8 sec/clus the FAT overhead leaves only ~65,404 clusters.
SEC_PER_CLUS_CANDIDATES = (8, 4, 2, 1)
RESERVED_SECTORS  = 32          # FAT32 requires ≥ 32 (boot + FSInfo + backup)
NUM_FATS          = 2
ROOT_CLUSTER      = 2           # root directory starts at cluster 2
FS_INFO_SECTOR    = 1
BACKUP_BOOT       = 6

# Must match disk_layout.rs after the Phase 00 update.
CELL_TABLE_BASE_LBA = 526_336


def fat32_geometry(total_sectors: int):
    """Return (sec_per_clus, fat_size_32, data_start_lba, data_clusters).

    Tries each candidate cluster size (largest first) and keeps the first one
    that satisfies the FAT32 65,525-cluster minimum. For each candidate the
    FAT size is iterated until it converges; FAT32 entries are 4 bytes each.
    """
    for spc in SEC_PER_CLUS_CANDIDATES:
        fat_size = 1
        for _ in range(32):
            data_region = total_sectors - RESERVED_SECTORS - NUM_FATS * fat_size
            clusters = data_region // spc
            # +2: cluster indices start at 2; entry 0 and 1 are reserved.
            new_fat = ((clusters + 2) * 4 + SECTOR_SIZE - 1) // SECTOR_SIZE
            if new_fat == fat_size:
                break
            fat_size = new_fat

        data_start = RESERVED_SECTORS + NUM_FATS * fat_size
        clusters = (total_sectors - data_start) // spc
        if clusters >= 65_525:
            return spc, fat_size, data_start, clusters

    raise SystemExit(
        f"[mkfat32] ERROR: no cluster size yields ≥ 65525 data clusters for "
        f"{total_sectors} sectors; volume too small for FAT32."
    )


def main():
    if len(sys.argv) not in (3, 4):
        raise SystemExit(
            "Usage: python mkfat32_inplace.py <image_path> <total_sectors> [base_lba]"
        )

    img_path      = sys.argv[1]
    total_sectors = int(sys.argv[2])
    # MBR layout (write-mbr.py): the FAT32 volume lives inside partition P1,
    # so all structures are offset by the partition start. Default 0 keeps the
    # legacy whole-disk behavior for kernel_fs.img-style images.
    base_lba      = int(sys.argv[3]) if len(sys.argv) == 4 else 0

    # Guard: stay clear of the cell bootstrap table.
    if base_lba + total_sectors > CELL_TABLE_BASE_LBA:
        raise SystemExit(
            f"[mkfat32] ERROR: base {base_lba} + total_sectors {total_sectors} would overlap "
            f"CELL_TABLE_BASE_LBA {CELL_TABLE_BASE_LBA}"
        )

    sec_per_clus, fat_size, data_start, clusters = fat32_geometry(total_sectors)

    # ── Boot sector (BPB at LBA 0) ─────────────────────────────────────────────
    boot = bytearray(SECTOR_SIZE)
    boot[0:3]   = b'\xEB\x58\x90'          # jump + NOP (FAT32 canonical offset)
    boot[3:11]  = b'MSWIN4.1'
    # Core BPB (offset 11-35)
    struct.pack_into('<H', boot, 11, SECTOR_SIZE)       # BytesPerSector
    boot[13]    = sec_per_clus                           # SectorsPerCluster
    struct.pack_into('<H', boot, 14, RESERVED_SECTORS)  # ReservedSectors
    boot[16]    = NUM_FATS                               # NumFATs
    struct.pack_into('<H', boot, 17, 0)                 # RootEntCnt = 0 (FAT32)
    struct.pack_into('<H', boot, 19, 0)                 # TotSec16 = 0 (FAT32 uses TotSec32)
    boot[21]    = 0xF8                                   # MediaType: fixed disk
    struct.pack_into('<H', boot, 22, 0)                 # FATSz16 = 0 (FAT32 uses FATSz32)
    struct.pack_into('<H', boot, 24, 63)                # SecPerTrk (irrelevant)
    struct.pack_into('<H', boot, 26, 255)               # NumHeads (irrelevant)
    struct.pack_into('<I', boot, 28, base_lba)          # HiddSec = partition start
    struct.pack_into('<I', boot, 32, total_sectors)     # TotSec32
    # FAT32 Extended BPB (offset 36-89)
    struct.pack_into('<I', boot, 36, fat_size)          # FATSz32
    struct.pack_into('<H', boot, 40, 0)                 # ExtFlags: both FATs active
    struct.pack_into('<H', boot, 42, 0)                 # FSVer: 0.0
    struct.pack_into('<I', boot, 44, ROOT_CLUSTER)      # RootClus = 2
    struct.pack_into('<H', boot, 48, FS_INFO_SECTOR)    # FSInfo sector
    struct.pack_into('<H', boot, 50, BACKUP_BOOT)       # BkBootSec
    # Reserved12 at offset 52-63 (zeros)
    boot[64]    = 0x80                                   # DrvNum
    boot[65]    = 0x00                                   # Reserved1
    boot[66]    = 0x29                                   # BootSig
    struct.pack_into('<I', boot, 67, 0x56494F44)        # VolID "VIOD"
    boot[71:82] = b'Cellos DATA'                        # VolLab (11 bytes)
    boot[82:90] = b'FAT32   '                           # FilSysType (8 bytes)
    boot[510:512] = b'\x55\xAA'

    # Backup boot sector at LBA BACKUP_BOOT (identical).
    boot_backup = bytes(boot)

    # ── FSInfo sector (LBA 1) ──────────────────────────────────────────────────
    fsinfo = bytearray(SECTOR_SIZE)
    struct.pack_into('<I', fsinfo, 0,   0x41615252)   # LeadSig
    struct.pack_into('<I', fsinfo, 484, 0x61417272)   # StrucSig
    # Free cluster count: clusters - 1 (cluster 2 = root dir = allocated).
    struct.pack_into('<I', fsinfo, 488, clusters - 1) # FreeCount
    struct.pack_into('<I', fsinfo, 492, 3)            # NxtFree: next free cluster
    struct.pack_into('<I', fsinfo, 508, 0xAA550000)   # TrailSig

    # ── FAT tables (both copies) ──────────────────────────────────────────────
    # Each entry is 4 bytes; only low 28 bits are data (top 4 bits are reserved).
    fat = bytearray(fat_size * SECTOR_SIZE)
    # FAT[0]: media descriptor (0x0FFFFF8) + reserved high nibble → 0xFFFFFFF8
    struct.pack_into('<I', fat, 0,  0xFFFFFFF8)
    # FAT[1]: end-of-chain mark
    struct.pack_into('<I', fat, 4,  0xFFFFFFFF)
    # FAT[2]: root directory cluster — end-of-chain (single cluster, no subdirs yet)
    struct.pack_into('<I', fat, 8,  0x0FFFFFFF)

    # ── Root directory cluster (zeroed = empty) ────────────────────────────────
    root_dir = bytearray(sec_per_clus * SECTOR_SIZE)

    # ── Write IN-PLACE (r+b — never extends the file) ─────────────────────────
    # All LBAs below are relative to the partition start (base_lba).
    with open(img_path, 'r+b') as f:
        # Boot sector at partition LBA 0
        f.seek(base_lba * SECTOR_SIZE)
        f.write(boot)
        # FSInfo at partition LBA 1
        f.seek((base_lba + FS_INFO_SECTOR) * SECTOR_SIZE)
        f.write(fsinfo)
        # Backup boot sector at partition LBA BACKUP_BOOT
        f.seek((base_lba + BACKUP_BOOT) * SECTOR_SIZE)
        f.write(boot_backup)
        # FAT1 at partition LBA RESERVED_SECTORS
        f.seek((base_lba + RESERVED_SECTORS) * SECTOR_SIZE)
        f.write(fat)
        # FAT2 at partition LBA RESERVED_SECTORS + fat_size
        f.seek((base_lba + RESERVED_SECTORS + fat_size) * SECTOR_SIZE)
        f.write(fat)
        # Root directory at cluster 2 data area
        root_lba = base_lba + data_start + (ROOT_CLUSTER - 2) * sec_per_clus
        f.seek(root_lba * SECTOR_SIZE)
        f.write(root_dir)

    print(
        f"[mkfat32] {img_path}: base=LBA {base_lba}, {total_sectors} sectors, "
        f"{clusters} data clusters (FAT32), FATsz32={fat_size}, "
        f"data start=LBA {base_lba + data_start}"
    )


if __name__ == '__main__':
    main()
