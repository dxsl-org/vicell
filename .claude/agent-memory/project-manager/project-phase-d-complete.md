---
name: phase-d-fat16-complete
description: Phase D: FAT16 Write Persistence complete 2026-06-03 — 5 sub-phases, 13/13 tests pass; BlkRead/BlkWrite + FAT16 formatter + BlockStream + /data/ routing working
metadata:
  type: project
---

## Phase D: FAT16 Write Persistence (Complete)

**Completion Date:** 2026-06-03  
**Duration:** Single working session  
**Test Status:** 13/13 integration tests pass ✅

### Sub-phases
1. **Phase 1 (BlkRead/BlkWrite Syscalls)** — exposed VirtIO block I/O via raw syscalls 500/501; kernel handlers in `ViCell_syscall_dispatch` numeric fallback
2. **Phase 2 (FAT16 Format)** — created `tools/mkfat16.py` formatter (81920 sectors, 8 sec/cluster, 10225 clusters); integrated into `gen_disk.ps1` step 3c
3. **Phase 3 (BlockStream + fatfs)** — created `/src/block_stream.rs` IoBase adapter; mounted FAT16 at VFS startup; verified zero unsafe in cell
4. **Phase 4 (VFS Routing)** — branched OP_WRITE and OP_READ on `/data/` prefix; `write_fat16`/`read_fat16` helpers; `/tmp/` unchanged (RamFS)
5. **Phase 5 (Integration Test)** — added `vfs_fat16_write_read` test; boot → write marker to `/data/test.txt` → read via vcat → assert

### Evidence
- `cargo check` passes for kernel + ostd + service-vfs
- `gen_disk.ps1` output: "10225 data clusters (FAT16), FATsz=40, data_start=LBA 113"
- Boot prints: "[vfs] FAT16 /data volume mounted"
- All existing tests still pass (cell table at LBA 82000 intact)
- Phase 5 test `vfs_fat16_write_read` **PASSES**: write + read round-trip confirmed

### Key Facts
- **Disk Geometry:** LBA 0–81919 (40 MB), before cell table at 82000
- **Cluster Count:** 10225 (FAT16 window: 4085–65524) ✅
- **Writing:** `/data/*` paths route through fatfs; `/tmp/*` unchanged (RamFS)
- **Block I/O:** Raw syscalls 500/501 (no stable ABI change — stayed private to ostd)
- **Mount:** Fallback to RamFS-only if FAT16 fails (graceful degradation)
- **Seeking:** End-seek not called during normal mount/create (BlockStream stub unused)

### Files Touched
**Created:**
- `tools/mkfat16.py`
- `cells/services/vfs/src/block_stream.rs`

**Modified:**
- `kernel/src/task/syscall.rs` (BlkRead/BlkWrite)
- `libs/ostd/src/syscall.rs` (sys_blk_read/write)
- `cells/services/vfs/Cargo.toml` (fatfs dep)
- `cells/services/vfs/src/main.rs` (mount + routing)
- `gen_disk.ps1` (mkfat16 step)
- `tests/integration/tests/boot.rs` (vfs_fat16_write_read)
- `docs/system-architecture.md` (status update)
- `docs/project-changelog.md` (Phase D entry)

### Next Phase
**Phase E (Planned):** Reboot persistence (graceful QEMU shutdown), subdirectories under `/data/`, OP_UNLINK for FAT16, sector-range clamp + capability gate for block syscalls, wider OP_WRITE for >255-byte content.
