---
title: "Phase G: FAT16 Completion"
description: "Close final FAT16 gaps — block-I/O capability flag, FAT16 rmdir, and two integration tests."
status: complete
priority: P2
effort: 4h
branch: main
tags: [fat16, vfs, capability, kernel, testing]
created: 2026-06-03
completed: 2026-06-03
---

# Phase G: FAT16 Completion

Phases C–F built a working FAT16 filesystem. Phase G closes the four remaining gaps:
capability-token replacement for the fragile `VFS_TASK_ID` hardcode, FAT16 directory
removal, and two missing integration tests (negative block-I/O + subdir reboot persistence).

## Scope Boundary

| In scope | Out of scope |
|----------|-------------|
| `can_block_io` TCB flag + spawn-time grant | Full cap-token system (Phase H) |
| OP_RMDIR for empty FAT16 dirs | Recursive rmdir (Phase H) |
| Negative block-I/O integration test | Formal security audit |
| Subdir reboot persistence test | Nested-dir OP_RMDIR persistence test |
| OP_RMDIR for nested `/data/` paths | OP_RMDIR for `/tmp` (RamFS already works) |

## Phases

| # | Phase | Status | Files | Depends on |
|---|-------|--------|-------|-----------|
| 1 | [can_block_io TCB flag](phase-01-can-block-io-flag.md) | ✅ complete | kernel: tcb.rs, loader.rs, syscall.rs | — |
| 2 | [OP_RMDIR for FAT16](phase-02-op-rmdir-fat16.md) | ✅ complete | vfs/main.rs | — |
| 3 | [Negative block-I/O test](phase-03-negative-block-io-test.md) | ✅ complete | shell: cmd_sys.rs, executor.rs; boot.rs | Phase 1 |
| 4 | [Subdir reboot persistence test](phase-04-subdir-persistence-test.md) | ✅ complete | boot.rs | — (code already correct) |

## Dependency Graph

```
Phase 1 (TCB flag)        ── independent
Phase 2 (OP_RMDIR)        ── independent (file: vfs/main.rs, no overlap)
Phase 3 (negative test)   ── depends on Phase 1 (exercises the new flag-based gate)
Phase 4 (persistence test)── independent (Phases E+F code already correct)
```

Apply order: **1 → 2 → 3 → 4**. Phase 3 must run after Phase 1 so the gate it asserts
against is the new flag-based one, not the hardcoded `VFS_TASK_ID`.

## File Ownership (no parallel-phase conflicts)

| File | Owning phase |
|------|-------------|
| `kernel/src/task/tcb.rs` | 1 |
| `kernel/src/loader.rs` | 1 |
| `kernel/src/task/syscall.rs` | 1 |
| `cells/services/vfs/src/main.rs` | 2 |
| `cells/apps/shell/src/cmd_sys.rs` | 3 |
| `cells/apps/shell/src/executor.rs` | 3 |
| `tests/integration/tests/boot.rs` | 3 and 4 (different test fns, no line overlap) |

Phases 1 and 2 touch disjoint crates → safe to apply in either order.
Phase 3 and 4 both append new `#[test]` fns to `boot.rs` — append-only, no shared lines.

## Verification Corrections vs. Input Context

The original brief contained three inaccuracies, corrected here after re-grep:

1. **`spawn_from_path` lives in `loader.rs:44`, NOT `task.rs`.** `task.rs:spawn_from_file`
   (line 268) is a stub that returns `Err(NotSupported)`. The real path-spawn entry is
   `crate::loader::spawn_from_path` (loader.rs:44), invoked by the `SpawnFromPath` syscall
   (syscall.rs:786). The `can_block_io` grant must be applied there, on the `tid` returned
   by `spawn_from_mem` (loader.rs:68).
2. **OP_RMDIR currently routes ONLY to `vfs.rmdir`** (main.rs:425-430) — confirmed correct.
   But note `unlink_fat16` (main.rs:315) already does `fs.root_dir().remove(rel)`, which in
   fatfs removes empty dirs too. Phase 2 reuses that semantics to stay DRY.
3. **`sys_blk_read(sector: u64, buf: &mut [u8; 512]) -> bool`** (ostd syscall.rs:63) takes a
   fixed-array reference. Phase 3's shell command must pass `&mut [0u8; 512]`. The existing
   reboot test waits for `"System shutting down"` after `shutdown` — Phase 4 mirrors that.

## Global Success Criteria

- `cargo check -p vios-kernel`, `-p service-vfs`, `-p app-shell`, and the integration crate all pass.
- `VFS_TASK_ID` constant fully removed; no remaining references.
- New integration tests `block_io_denied_non_vfs` and `vfs_fat16_subdir_persistence` pass
  under QEMU (or SKIP cleanly when prerequisites absent).
- Existing tests `vfs_fat16_reboot_persistence`, `vfs_fat16_write_read`, `boots_to_shell_prompt`
  still pass (regression guard — VFS must still receive its block-I/O grant).

## Unresolved Questions

See per-phase files; the only cross-cutting one: confirm VFS is always spawned via
`SpawnFromPath` with a path ending `/bin/vfs` (Phase 1, Risk R1-1).
