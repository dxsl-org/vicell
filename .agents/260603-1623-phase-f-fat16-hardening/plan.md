---
title: "Phase F: FAT16 Hardening"
description: "Widen OP_WRITE, add /data unlink + subdirs, gate block syscalls to the VFS cell"
status: complete
priority: P2
effort: 5h
branch: main
tags: [vfs, fat16, ipc, syscall, security]
created: 2026-06-03
completed: 2026-06-03
---

# Phase F: FAT16 Hardening

Tightens four deferred items left over from Phases C–E now that `/data/` FAT16
read/write/persistence works. Each phase is independently shippable and
testable in a single QEMU boot.

## Phases

| # | Phase | Status | Effort | Files |
|---|-------|--------|--------|-------|
| 1 | [OP_WRITE 4-byte header widening](phase-01-op-write-widening.md) | complete | 1h | `cmd_fs.rs`, `vfs/main.rs` (OP_WRITE arm) |
| 2 | [OP_UNLINK for /data/ (FAT16)](phase-02-op-unlink-fat16.md) | complete | 45m | `vfs/main.rs` (OP_UNLINK arm + helper) |
| 3 | [Subdirectories under /data/](phase-03-subdirectories.md) | complete | 2h | `vfs/main.rs` (fat16 helpers + OP_MKDIR arm) |
| 4 | [Block syscall capability gate](phase-04-block-syscall-cap-gate.md) | complete | 1h | `kernel/src/task/syscall.rs` (BlkRead/Write/Flush arms) |

## Dependency Graph

```
Phase 1 ─┐
Phase 2 ─┼─ all independent in different file regions
Phase 4 ─┘   (Phase 1, 2 in vfs/main.rs disjoint sections; 4 in kernel)
Phase 3 ── depends on Phase 2 (refactors unlink_fat16 into the traversal helper)
```

Apply order: **1 → 2 → 3 → 4**. Phases 1 and 3 both touch `vfs/main.rs` but in
disjoint regions; do them sequentially to avoid merge churn.

## Key Verified Facts (re-grepped 2026-06-03)

- Shell writer is `cmd_fs.rs:write_file` (cmd_fs.rs:263), called from
  `executor.rs:93` on a `Redirect::StdoutTo`. Cap today: `253 - pl` (cmd_fs.rs:266),
  256-byte client buffer (cmd_fs.rs:267).
- Server OP_WRITE arm at `vfs/main.rs:340-358`, 3-byte header, 512-byte `buf`
  (vfs/main.rs:292), cap `3 + pl + cl <= 512`.
- `OP_WRITE=4, OP_MKDIR=5, OP_RMDIR=6, OP_UNLINK=7, OP_READ=8` (vfs/main.rs:35-39,
  cmd_fs.rs:16-18,256-257). **No opcode conflict** — OP_READ=8, OP_MKDIR=5.
- `write_fat16`/`read_fat16` flat-root only (vfs/main.rs:248-286): strip
  `/data/`, call `root.create_file(name)` / `root.open_file(name)`.
- `OP_UNLINK` arm routes only to `vfs.unlink` (RamFS) — no FAT16 branch
  (vfs/main.rs:383-388).
- Block syscalls: `ViCell_syscall_dispatch` constructs the enum in the numeric
  fallback (syscall.rs:1237-1240); **`caller_id` is computed AFTER the match
  at syscall.rs:1249** and is the first param of `handle_syscall`. The actual
  dispatch arms are `BlkRead`/`BlkWrite`/`BlkFlush` at syscall.rs:1095/1112/1072.
  The gate MUST go in `handle_syscall`, not the fallback. [CORRECTION to brief]
- fatfs API: `Dir::create_dir(&self, path) -> Result<Dir>`,
  `open_dir -> Result<Dir>`, `create_file -> Result<File>`. **No `into_dir()`**
  — `create_dir` already returns a `Dir`. [CORRECTION to brief]
  Paths are '/'-separated and traversed natively, but intermediate dirs are NOT
  auto-created — Phase 3 creates each component.
- Integration tests live in `tests/integration/tests/boot.rs`; Phase C/D/E
  patterns at lines 303/329/365. Harness verbs: `send_line`, `wait_for`,
  `output_contains`, `dump`.
- `VFS = task 3` boot order (cmd_fs.rs:11-15, syscall.rs:657-671 ServiceLookup).

## Scope Boundary

| In scope | Out of scope (Phase G) |
|----------|------------------------|
| 4-byte OP_WRITE header (u16 content_len) | Streaming / chunked multi-message writes |
| OP_UNLINK for /data/ flat files | OP_UNLINK for /data/ subdirectories |
| /data/ subdirs (create + write + read) | FAT16 OP_RMDIR |
| Block I/O caller_id==3 check | Formal capability token for block syscalls |
| Same-boot integration tests | Reboot persistence of subdir structure |

## Cross-Phase Risks

| Risk | L | I | Mitigation |
|------|---|---|-----------|
| 4-byte header breaks existing tests | Low | High | All existing content ≤253 bytes; widening is additive. Existing Phase C/D/E tests assert behavior, not wire bytes — re-run them. |
| Hardcoded task ID 3 breaks if boot order shifts | Med | High | `log::warn!` on rejection; comment cross-refs ServiceLookup (syscall.rs:663) which also hardcodes 3. |
| fatfs subpath semantics differ from assumption | Low | Med | Verified via docs.rs: paths are '/'-separated, intermediate dirs NOT auto-created. Phase 3 creates components explicitly + mkdir test. |

## Definition of Done (all phases)

- `cargo check -p app-shell -p service-vfs` and `cargo check -p kernel` clean.
- `cargo clippy -- -D warnings` clean on touched crates.
- All existing `boot.rs` tests still pass (no regression).
- New tests in `boot.rs` for Phases 1–3 pass; Phase 4 verified by no-regression
  + code review (negative test deferred — see phase-04).
