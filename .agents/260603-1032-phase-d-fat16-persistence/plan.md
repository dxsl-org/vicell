---
title: "Phase D: FAT16 Write Persistence"
description: "Route /data/* paths through a FAT16 filesystem on the VirtIO block device so files survive reboot."
status: complete
priority: P2
effort: 9h
branch: main
tags: [vfs, fat16, persistence, virtio-blk, syscall]
created: 2026-06-03
completed: 2026-06-03
---

# Phase D: FAT16 Write Persistence

Make VFS writes persistent. Phase C added volatile RamFS writes under `/tmp/`.
Phase D routes `/data/*` paths through a FAT16 filesystem laid down on the
VirtIO block device (`disk_v3.img`, LBA 0–81919), so files survive reboot.

## Architecture (verified data flow)

```
shell ──IPC OP_WRITE──▶ VFS cell ──/data/?──┬─ yes ─▶ write_fat16() ─▶ fatfs ─▶ BlockStream
                                            │                                      │
                                            └─ /tmp/ ─▶ RamFS (volatile)           │ sys_blk_write(500/501)
                                                                                   ▼
                                                            kernel ViCell_syscall_dispatch (raw 500/501)
                                                                                   │
                                                                                   ▼
                                                            viVirtIOBlk.write_sector() ─▶ QEMU VirtIO disk
```

VFS already serves `/bin/*` from embedded RamFS (`include_bytes!`); `/tmp/` and
`/data/` are the only writable trees. `/data/` is flat (root dir only) in Phase D.

## Phases

| # | Phase | Status | Effort | Blockers |
|---|-------|--------|--------|----------|
| 1 | [Kernel block I/O syscalls (raw 500/501)](phase-01-blk-syscalls.md) | complete | 2h | none |
| 2 | [Format FAT16 region in gen_disk.ps1](phase-02-format-fat16.md) | complete | 2h | none |
| 3 | [BlockStream adapter + fatfs in VFS cell](phase-03-blockstream-fatfs.md) | complete | 2h | P1 |
| 4 | [Route /data/* to FAT16](phase-04-vfs-fat16-routing.md) | complete | 2h | P3 |
| 5 | [Integration test (same-session write+read)](phase-05-integration-test.md) | complete | 1h | P2,P4 |

Phases 1 and 2 are independent (different files) and may proceed in parallel.
Phase 3 depends on Phase 1 (needs `sys_blk_read`/`sys_blk_write` in ostd).
Phase 4 depends on Phase 3 (needs the mounted `fat_fs`).
Phase 5 depends on Phase 2 (formatted disk) and Phase 4 (routing live).

## Key corrections to the original research (verified against codebase)

1. **`virtio_blk` has NO free `read_sector`/`write_sector` functions.** They are
   methods on the `viVirtIOBlk` ZST via the `ViBlockDevice` trait
   (`kernel/src/task/drivers/virtio_blk.rs:101-152`). The kernel handler must
   call `viVirtIOBlk.read_sector(sector, buf)`, not `virtio_blk::read_sector(...)`.

2. **Disk image is 81920 sectors (40 MB), not 82000.** `gen_disk.ps1:108`
   creates a blank 81920-sector image; `write-cell-table.py` then *extends* it so
   the cell table header lands at LBA 82000. **FAT16 must use ≤ 81920 sectors** to
   stay clear of both the pad zone (81920–81999) and the cell table at LBA 82000.
   Plan uses **81920 sectors** for the FAT16 region.

3. **Raw syscall integration point is the `_ => match syscall_id` fallback** in
   `ViCell_syscall_dispatch` (`kernel/src/task/syscall.rs:1158`). `ViSyscall::from(500)`
   returns `ViSyscall::Unknown` (`libs/api/src/syscall.rs:139`), so 500/501 fall
   through to the numeric match. Add `BlkRead`/`BlkWrite` variants to the internal
   `Syscall` enum and map 500/501 there. `libs/api/src/syscall.rs` is NOT touched
   (Law 1 avoided — no 2x confirmation needed).

4. **DRY win for the formatter:** `tools/mkfat32.py` is already a complete, proven
   FAT16 BPB writer (despite its name). `tools/mkfat16.py` adapts it to write
   in-place at LBA 0 of an existing image with an empty root dir.

## Cross-cutting risks (per-phase risks in phase files)

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| FAT16 region overlaps cell table at LBA 82000 | Medium | High | Cap FAT16 at 81920 sectors; assert in mkfat16.py |
| `fatfs::FileSystem::new()` panics on bad BPB | Medium | High | Wrap in match; fall back to RamFS-only, log warning |
| `viVirtIOBlk` not probed (no disk) | Low | Low | `read_sector` returns `Err`; syscall returns 0; VFS skips mount |
| ostd raw syscall unsafe unlabeled | Low | Low | Add `// SAFETY:` per Law 4 |

## Out of scope (Phase E)

Reboot persistence test (needs QEMU graceful shutdown), subdirectories under
`/data/`, OP_MKDIR/OP_UNLINK for FAT16, runtime format-on-first-boot, FAT32,
formal `ViSyscall` enum entries for block I/O.

## Unresolved questions

See bottom of `phase-01` and `phase-03`.
