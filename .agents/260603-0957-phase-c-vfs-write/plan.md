---
title: "Phase C: VFS Write + Shell Redirect"
description: "Enable RamFS file writes (OP_WRITE) and echo > /tmp redirect so shell output persists in-session"
status: complete
priority: P2
effort: 5h
branch: main
tags: [vfs, shell, ipc, ramfs, redirect, phase-c]
created: 2026-06-03
completed: 2026-06-03
---

# Phase C: VFS Write + Shell Redirect

## Goal
Enable file creation/writing in the VFS RamFS backing (`OP_WRITE`, opcode 4) and wire shell
output redirect so `echo TEXT > /tmp/file` persists `TEXT` in-session. Reading back via `cat`
must return the written bytes. FAT32/disk persistence is Phase D.

## CRITICAL — Research Corrections (verified against codebase 2026-06-03)

The originating research contained two plan-breaking errors. This plan supersedes them:

1. **`echo` is NOT a shell built-in.** No `cmd_echo` exists anywhere (`Grep cmd_echo` → 0 matches).
   `echo` falls through `dispatch_builtin` (executor.rs:149) to `spawn_external`, which spawns
   `/bin/echo`. That embedded binary (`cells/apps/utils/src/bin/echo.rs:11`) ignores argv and
   only prints `"Echo (External App): Hello!"`. **We cannot capture echo by calling a built-in
   that does not exist.** → Phase 3 ADDS a real `echo` built-in.

2. **`VFS_ENDPOINT = 2` is wrong, but `= 3` is fragile.** A `ServiceLookup` syscall (#100) already
   exists in the kernel (`syscall.rs:191, 640, 1163`) and returns `vfs=3` by name. There is **no
   ostd wrapper** for it yet. → Phase 1 fixes the constant to 3 AND adds a `sys_service_lookup`
   ostd wrapper, using dynamic lookup with a hardcoded `3` fallback.

3. **VFS recv buffer is 512 bytes, not 256.** `let mut buf = [0u8; 512]` (main.rs:205). The
   256-byte client buffer in cmd_fs.rs is the binding constraint, not the server. OP_WRITE
   handler must read `buf[0]`/`buf[1]`/path from the 512-byte buffer correctly.

4. **VFS path traversal stores children by basename, no `/tmp/` string match needed for write.**
   `find_node`/`find_node_mut` split on `/`. A write to `/tmp/test.txt` needs the parent `/tmp`
   to exist (it does — main.rs:79) and inserts child `test.txt`. The `/tmp/` prefix guard is a
   security gate, applied as a string check before traversal.

## Phases

| # | Phase | File | Status | Effort | Blocks |
|---|-------|------|--------|--------|--------|
| 1 | Fix VFS endpoint + add ServiceLookup wrapper | [phase-01](phase-01-fix-vfs-endpoint.md) | complete | 1h | — |
| 2 | Implement OP_WRITE in VFS (RamFS write_file) | [phase-02](phase-02-vfs-op-write.md) | complete | 1.5h | — |
| 3 | Add echo built-in + stdout redirect capture | [phase-03](phase-03-shell-echo-redirect.md) | complete | 1.5h | 1, 2 |
| 4 | Integration test (echo > file > cat) | [phase-04](phase-04-integration-test.md) | complete | 1h | 1,2,3 |

## Dependency Graph

```
Phase 1 (endpoint+wrapper) ─┐
                            ├─→ Phase 3 (echo built-in + redirect) ─→ Phase 4 (integration test)
Phase 2 (OP_WRITE handler) ─┘
```

Phases 1 and 2 are independent (different crates: `app-shell` + `ostd` vs `service-vfs`) and may
run in parallel. Phase 3 depends on both. Phase 4 depends on all.

## File Ownership (no parallel conflicts)

| Phase | Owns (writes) |
|-------|---------------|
| 1 | `libs/ostd/src/syscall.rs`, `cells/apps/shell/src/cmd_fs.rs` (const + new fn) |
| 2 | `cells/services/vfs/src/main.rs` |
| 3 | `cells/apps/shell/src/commands.rs`, `cells/apps/shell/src/executor.rs`, `cells/apps/shell/src/cmd_fs.rs` (write_file fn) |
| 4 | `tests/integration/tests/boot.rs` |

Phase 1 and Phase 3 both touch `cmd_fs.rs` — Phase 3 depends on Phase 1, so they run sequentially.
No true parallel conflict.

## Scope Boundary

| In scope | Out of scope |
|----------|-------------|
| RamFS `write_file` for `/tmp/` paths | FAT32/disk persistence (Phase D) |
| `OP_WRITE` opcode 4 handler | OP_WRITE for paths outside `/tmp/` (rejected) |
| `echo` as a real shell built-in | Fixing `/bin/echo` external binary (separate) |
| `>` (StdoutTo) redirect for built-ins | `>>` append, `<` stdin, `2>` stderr redirect |
| Dynamic ServiceLookup + hardcoded fallback | Dynamic service registry (v0.3) |
| Capture built-in output to Vec<u8> | External process output capture (needs pipe caps, Phase 17a) |
| Integration test: echo > file > cat | Multi-KB writes (>253 byte payload chunking) |

## Key Dependencies
- Phase A/B IPC fix (`ipc_recv` wakes senders) — required for VFS request/reply. Verified present.
- `/tmp` dir exists in RamFS root (main.rs:79). Required as write parent.
- Test harness API (`boot`, `wait_for`, `send_line`, `dump`) — verified in `tests/integration/src/lib.rs`.

## Success Criteria (measurable)
1. `cargo check` passes for `ostd`, `service-vfs`, `app-shell` on `riscv64gc-unknown-none-elf`.
2. In QEMU: `echo PHASE_C_WRITE > /tmp/test.txt` returns to prompt with no error line.
3. `cat /tmp/test.txt` prints `PHASE_C_WRITE`.
4. `echo X > /etc/passwd` (outside /tmp) prints a redirect error and does NOT write.
5. Integration test `vfs_write_echo_redirect` passes.

## Unresolved Questions
See bottom of phase-03 and phase-04.
