---
phase: 1
title: "Fix VFS endpoint + add ServiceLookup ostd wrapper"
status: complete
priority: P1
effort: 1h
dependencies: []
completed: 2026-06-03
---

# Phase 1: Fix VFS Endpoint + ServiceLookup Wrapper

## Context Links
- `cells/apps/shell/src/cmd_fs.rs:10` — `const VFS_ENDPOINT: usize = 2;` (WRONG)
- `kernel/src/task/syscall.rs:648-662` — spawn-order comment + ServiceLookup handler (authoritative: vfs=3)
- `kernel/src/task/syscall.rs:1163` — `100 => Syscall::ServiceLookup` (syscall number 100)
- `libs/ostd/src/syscall.rs` — ostd syscall wrappers (no ServiceLookup wrapper exists yet)

## Overview
- **Priority:** P1 (everything else depends on shell reaching the VFS cell)
- **Status:** pending
- **Description:** Shell's `VFS_ENDPOINT = 2` targets task 2 (`user_hello` smoke-test), not the VFS
  service (task 3). All `mkdir`/`rmdir`/`rm` IPC is silently misrouted today. Fix the constant to
  3 and add a `sys_service_lookup` ostd wrapper so the endpoint is resolved dynamically (hardcoded
  3 as fallback), per the kernel's own ServiceLookup contract.

## Key Insights
- VERIFIED: kernel spawns `init=1`, `user_hello=2`, then init spawns `vfs=3, config=4, input=5,
  net=6, compositor=7, shell=8` (`syscall.rs:649-651`, "Verified from QEMU serial log").
- VERIFIED: `ServiceLookup` is syscall #100 (`syscall.rs:1163`), takes `(name_ptr, name_len)`,
  returns the task id for `"vfs"|"config"|"input"|"net"|"compositor"|"shell"`, else `FileNotFound`.
- VERIFIED: no `sys_service_lookup` / `ServiceLookup` reference exists in ostd or any cell
  (`Grep ServiceLookup **/*.rs` → only kernel hits). The wrapper must be added.
- The hardcoded-3 fix alone resolves the immediate bug; the wrapper is added so Phase 3's redirect
  and future cells don't re-hardcode. KISS: thin wrapper, no registry.

## Requirements
**Functional**
- Shell IPC to VFS reaches task 3.
- `sys_service_lookup(name: &str) -> Option<usize>` returns the cell id, `None` on not-found.
- `VFS_ENDPOINT` resolution: try `sys_service_lookup("vfs")`, fall back to literal `3`.

**Non-functional**
- No change to `libs/api/` ABI (Law 1) — ServiceLookup syscall number already fixed in kernel.
- `#![forbid(unsafe_code)]` respected in shell cell; ostd wrapper uses existing `syscall()` shim.

## Architecture
**Data flow:**
```
shell cmd_fs / executor
  → vfs_endpoint()                    (new helper, cmd_fs.rs)
      → sys_service_lookup("vfs")     (new ostd wrapper → syscall #100)
          → kernel ServiceLookup → returns 3
      → on Err: fallback usize 3
  → sys_send(endpoint, msg)
```

The ServiceLookup syscall returns the id in the syscall return register. ostd's existing
`syscall(ViSyscall::…, a0, a1, …)` already returns `usize`; mirror an existing wrapper that maps
the return into `SyscallResult` / `Result`.

## Related Code Files
**Modify**
- `libs/ostd/src/syscall.rs` — add `sys_service_lookup`. First confirm `ViSyscall::ServiceLookup`
  variant exists in the ostd-side enum; if the enum is shared with kernel via `libs/api`, the
  variant + number 100 must be present. If ostd's `ViSyscall` lacks it, add the variant mapping to
  100 (matching `kernel/src/task/syscall.rs:1163`).
- `cells/apps/shell/src/cmd_fs.rs:9-10` — change const + add `vfs_endpoint()` helper; replace the
  two `sys_send(VFS_ENDPOINT, …)` call sites in `vfs_path_op` to use `vfs_endpoint()`.

**Create** — none.
**Delete** — none.

## Implementation Steps
1. Inspect `libs/ostd/src/syscall.rs` for the `ViSyscall` enum definition (or its `use` import).
   Confirm whether a `ServiceLookup` variant + discriminant 100 exists. If shared from `libs/api`,
   verify there; touching `libs/api` requires 2x user confirmation (Law 1) — flag if needed.
2. Add ostd wrapper, mirroring the style of `sys_open` (returns `Result<usize, SyscallError>`):
   ```rust
   /// Resolve a service cell's task id by name (e.g. "vfs" -> 3).
   /// Returns None if the service is not registered.
   pub fn sys_service_lookup(name: &str) -> Option<usize> {
       let ret = unsafe {
           syscall(ViSyscall::ServiceLookup, name.as_ptr() as usize, name.len(), 0, 0)
       };
       // Kernel returns FileNotFound (negative/err sentinel) when unknown.
       decode_lookup(ret)
   }
   ```
   Match the exact error-decoding convention used by neighbouring wrappers (`sys_open` at
   `syscall.rs:212`) — do not invent a new sentinel scheme.
3. In `cmd_fs.rs`, replace line 10:
   ```rust
   /// VFS service cell endpoint. Resolved dynamically via ServiceLookup with a
   /// hardcoded fallback of 3 (boot order: init=1, user_hello=2, vfs=3 — see
   /// kernel/src/task/syscall.rs:649). The previous value `2` targeted the
   /// user_hello smoke-test task and silently dropped all VFS writes.
   const VFS_ENDPOINT_FALLBACK: usize = 3;

   fn vfs_endpoint() -> usize {
       syscall::sys_service_lookup("vfs").unwrap_or(VFS_ENDPOINT_FALLBACK)
   }
   ```
4. Update `vfs_path_op` (cmd_fs.rs:47) to use `let ep = vfs_endpoint();` then `sys_send(ep, …)`.
5. `cargo check -p app-shell --target riscv64gc-unknown-none-elf`
6. `cargo check -p ostd --target riscv64gc-unknown-none-elf` (or whatever ostd's target is).

## Todo List
- [ ] Confirm `ViSyscall::ServiceLookup` variant + discriminant 100 in ostd-visible enum
- [ ] Add `sys_service_lookup` wrapper to `libs/ostd/src/syscall.rs`
- [ ] Replace `VFS_ENDPOINT = 2` with fallback const + `vfs_endpoint()` helper
- [ ] Route `vfs_path_op` sends through `vfs_endpoint()`
- [ ] `cargo check -p ostd` passes
- [ ] `cargo check -p app-shell` passes

## Success Criteria
- `cargo check` passes for both crates on the riscv64 target.
- `vfs_endpoint()` returns 3 in the default boot (verified by Phase 4 mkdir/rm working again).
- No `libs/api` ABI change unless explicitly confirmed.

## Risk Assessment
| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| `ViSyscall` enum lives in `libs/api` (ABI) | Medium | High | If so, the variant+100 likely already there since kernel maps it. Read first; only modify with 2x confirm (Law 1). If absent, prefer hardcoded-3 fallback alone and defer wrapper. |
| ServiceLookup return-value decoding differs from sys_open | Medium | Medium | Read kernel `Ok(id)` path (syscall.rs:653-662) + how Ok(usize) is marshalled to userspace; mirror exact convention. |
| Other cells also hardcode endpoint 2 | Low | Low | Grep confirmed only cmd_fs.rs:10 holds `VFS_ENDPOINT`. No other hits. |

## Security Considerations
- ServiceLookup exposes task ids by name — already kernel-gated to a fixed allowlist
  (syscall.rs:653-660). No new attack surface.
- Endpoint resolution failure falls back to 3, not 0; sending to a wrong id at worst drops the
  message (no privilege escalation in SAS + LBI).

## Evidence (Phase Complete)
- `cells/apps/shell/src/cmd_fs.rs:10` changed from `VFS_ENDPOINT = 2` to explanation comment + `VFS_ENDPOINT_FALLBACK = 3`
- `cells/apps/shell/src/cmd_fs.rs` added `vfs_endpoint()` helper function with `sys_service_lookup("vfs")` call
- `vfs_path_op()` function updated to use `vfs_endpoint()` instead of hardcoded constant
- `cargo check -p app-shell --target riscv64gc-unknown-none-elf` → exit 0 ✅

## Next Steps
- Unblocks Phase 3 (shell redirect uses `vfs_endpoint()` to send OP_WRITE).
- Independent of Phase 2 (different crate).
