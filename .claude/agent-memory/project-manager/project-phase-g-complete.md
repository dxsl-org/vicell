---
name: project-phase-g-complete
description: Phase G FAT16 Completion — 4 sub-phases complete, 19/19 integration tests pass
metadata:
  type: project
---

**Phase G Complete: 2026-06-03**

Four independent sub-phases closed all FAT16 feature gaps and added two key security/durability tests.

## Sub-phases
1. **`can_block_io` TCB flag** — Replaced `VFS_TASK_ID == 3` hardcode with per-cell flag; grants at spawn time when path ends `/bin/vfs`
2. **OP_RMDIR for FAT16** — Extended OP_RMDIR to route `/data/` paths to FAT16 (reuses `unlink_fat16` for DRY)
3. **Negative block-I/O test** — Shell command `blktest` + integration test asserting non-VFS cells denied
4. **Subdir persistence test** — Proves `/data/pdir/` writes survive reboot; mirrors Phase E pattern

## Evidence
- All 4 phases compile: `cargo build -p vios-kernel -p service-vfs -p app-shell -r` succeeds
- `VFS_TASK_ID` constant fully removed (0 grep hits)
- 19/19 integration tests pass (2 new tests appended to boot.rs; 17 existing tests still pass)
- Phase files updated with Evidence sections

## Files Modified
- `kernel/src/task/tcb.rs` — `can_block_io` field + default
- `kernel/src/loader.rs` — grant logic
- `kernel/src/task/syscall.rs` — `caller_has_block_io()` helper + gate updates
- `cells/services/vfs/src/main.rs` — OP_RMDIR branch
- `cells/apps/shell/src/cmd_sys.rs` — `cmd_blkio_test()`
- `cells/apps/shell/src/executor.rs` — dispatch registration
- `tests/integration/tests/boot.rs` — 2 new integration tests

## Documentation Updated
- `docs/system-architecture.md` — version bump to 0.2.1-dev (Phase G complete)
- `docs/project-changelog.md` — full Phase G entry with impact summary
- Plan file — status: complete
- All 4 phase files — Evidence sections added

## Impact
- Block I/O syscalls now capability-gated via per-cell flag (not boot-order-dependent ID)
- FAT16 rmdir + nested mkdir enable full directory lifecycle
- Security regression test locks in privilege separation
- Subdir persistence validated end-to-end; FAT16 is durable storage backend
