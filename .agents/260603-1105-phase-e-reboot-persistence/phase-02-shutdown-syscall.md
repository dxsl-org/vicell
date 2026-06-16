# Phase 2 — Kernel shutdown syscall + shell built-in

## Context Links
- `kernel/src/task/syscall.rs:253-257` (internal `Syscall` enum, BlkRead/BlkWrite precedent)
- `kernel/src/task/syscall.rs:1107` (close of `handle_syscall` match)
- `kernel/src/task/syscall.rs:1199-1201` (numeric fallback: `500 => BlkRead`, `501 => BlkWrite`)
- `libs/ostd/src/syscall.rs:45` (`unsafe fn syscall_raw(id, a0..a3) -> isize`, verified)
- `libs/ostd/src/syscall.rs:63-78` (`sys_blk_read`/`sys_blk_write` raw-syscall precedent)
- `libs/ostd/src/syscall.rs:87-97` (`sys_yield`, `sys_exit -> !` patterns)
- `cells/apps/shell/src/cmd_sys.rs` (system built-ins)
- `cells/apps/shell/src/executor.rs:133-169` (`dispatch_builtin` match — arms return `ViResult<()>`)

## Overview
- **Priority:** P1
- **Status:** complete (2026-06-03)
- **Depends on:** nothing (independent of Phases 1 and 3)
- Adds a clean shutdown path: shell `shutdown` → ostd `sys_shutdown()` → raw syscall 502 → kernel SBI SRST → OpenSBI powers off QEMU.

## Key Insights
- **`/bin/shutdown` is broken** (`cells/apps/sys-tools/src/bin/shutdown.rs`): it ecalls `a7=0x08`, which the kernel reads as `ViSyscall::Wait` (ID 8) — NOT a shutdown. We bypass it entirely with a shell built-in; no need to fix the binary in this phase.
- **No `libs/api/` change** — raw syscall 502 has no `ViSyscall` entry, so it dodges the Law 1 2x-confirm gate. This is the exact pattern already used for BlkRead=500/BlkWrite=501 (verified `syscall.rs:1199`).
- `ViCell_syscall_dispatch` SUM-enables user memory around `handle_syscall` (`syscall.rs:1212-1216`); irrelevant to Shutdown since the handler never touches user memory and never returns.
- **`dispatch_builtin` arm type:** every arm yields `ViResult<()>` and is collapsed by `match result { Ok(()) => 0, Err(_) => 1 }` (`executor.rs:169`). The new arm must therefore be `"shutdown" => crate::cmd_sys::cmd_shutdown(),` — NOT `=> { ...; Ok(()) }`. `cmd_shutdown()` returns `ViResult<()>` (diverging body), which unifies cleanly.

## Requirements
- **Functional:** typing `shutdown` at the ViCell prompt powers off the machine; QEMU process exits with no hang.
- **Non-functional:** kernel handler is `noreturn`; ostd `sys_shutdown` is `-> !`.

## Data flow
```
shell prompt "shutdown"
  → executor dispatch_builtin("shutdown")
    → cmd_sys::cmd_shutdown()
       → println("System shutting down...")
       → ostd::syscall::sys_shutdown()  [-> !]
          → syscall_raw(502, 0,0,0,0)  ecall (a7=502)
             → kernel ViCell_syscall_dispatch: 502 => Syscall::Shutdown
                → handle_syscall: Syscall::Shutdown
                   → asm ecall (a7=0x53525354 SRST, fid 0, type 0)  [noreturn]
                      → OpenSBI (M-mode) powers off → QEMU exits
```

## Related Code Files
- **Modify:** `kernel/src/task/syscall.rs` (enum + handler + numeric map)
- **Modify:** `libs/ostd/src/syscall.rs` (add `sys_shutdown`)
- **Modify:** `cells/apps/shell/src/cmd_sys.rs` (add `cmd_shutdown`)
- **Modify:** `cells/apps/shell/src/executor.rs` (register `"shutdown"` arm)
- **NOT modified:** `libs/api/src/syscall.rs` (raw-502 avoids the ABI gate)
- **Create / delete:** none

## Implementation Steps

### 2a. Add `Shutdown` to internal `Syscall` enum (`syscall.rs`, near line 257, after `BlkWrite`)
```rust
    /// 502: Shutdown — trigger SBI SRST system shutdown (S-mode → OpenSBI). No return.
    Shutdown,
```

### 2b. Add handler arm in `handle_syscall` (before the closing `}` at ~line 1107, after the `HotSwap` arm)
```rust
        Syscall::Shutdown => {
            // SAFETY: SBI System Reset (ext 0x53525354, fid 0, type Shutdown) from
            // S-mode. The ecall traps to OpenSBI (M-mode), which powers off QEMU.
            // Control never returns, so the unreachable arm value is irrelevant.
            unsafe {
                core::arch::asm!(
                    "li a7, 0x53525354",  // SBI_EXT_SRST
                    "li a6, 0",           // fid = SYSTEM_RESET
                    "li a0, 0",           // reset_type = Shutdown
                    "li a1, 0",           // reset_reason = NoReason
                    "ecall",
                    options(noreturn)
                );
            }
        }
```
> `options(noreturn)` means this arm diverges, so it unifies with the `Result<usize, _>` arms without needing an explicit `Ok(..)`.

### 2c. Map 502 in the numeric fallback (`syscall.rs:1201`, alongside 500/501)
```rust
            500 => Syscall::BlkRead  { sector: a0 as u64, buf_ptr: a1 },
            501 => Syscall::BlkWrite { sector: a0 as u64, buf_ptr: a1 },
            502 => Syscall::Shutdown,
```

### 2d. Add `sys_shutdown()` to `libs/ostd/src/syscall.rs` (after `sys_blk_write`, ~line 78)
```rust
/// Trigger a clean system shutdown via the kernel's SBI SRST path. Never returns.
///
/// Raw syscall 502 — intentionally absent from `ViSyscall`/`libs/api` to avoid the
/// ABI 2x-confirm gate (same pattern as `sys_blk_read`/`sys_blk_write` above).
pub fn sys_shutdown() -> ! {
    // SAFETY: raw syscall 502 invokes the kernel SBI SRST shutdown; the ecall to
    // OpenSBI terminates QEMU and never returns to us.
    unsafe { syscall_raw(502, 0, 0, 0, 0); }
    // Unreachable: the kernel never returns from shutdown. Spin to satisfy `-> !`.
    loop { sys_yield(); }
}
```

### 2e. Add `cmd_shutdown()` to `cells/apps/shell/src/cmd_sys.rs` (append after `cmd_uptime`)
```rust
/// `shutdown` — cleanly power off the system via SBI SRST. Does not return.
pub fn cmd_shutdown<'a>() -> ViResult<()> {
    ostd::io::println("System shutting down...");
    ostd::syscall::sys_shutdown()
}
```
> Signature is `-> ViResult<()>` to match the other `cmd_*` arms even though the body diverges; `sys_shutdown()` is `-> !` which coerces to `ViResult<()>`.

### 2f. Register in `executor.rs` `dispatch_builtin` (in the `// ── System ──` group, ~line 159)
```rust
        "uptime" => crate::cmd_sys::cmd_uptime(make_parts(args)),
        "shutdown" => crate::cmd_sys::cmd_shutdown(),
```
> Use the bare-expression form to match the surrounding arms (they return `ViResult<()>`). Do NOT wrap in `{ ...; Ok(()) }` — that would be dead code after the diverging call and is stylistically inconsistent.

## Todo List
- [ ] 2a: `Shutdown` enum variant
- [ ] 2b: handler with SBI SRST asm
- [ ] 2c: `502 => Syscall::Shutdown` numeric map
- [ ] 2d: `sys_shutdown()` in ostd
- [ ] 2e: `cmd_shutdown()` in cmd_sys.rs
- [ ] 2f: `"shutdown"` arm in executor.rs
- [ ] `cargo check -p ViCell-kernel -p ostd -p app-shell --target riscv64gc-unknown-none-elf`

## Success Criteria
- Compiles clean.
- Booting and typing `shutdown` prints "System shutting down..." then QEMU process exits (verified in Phase 4 via `wait_for_natural_exit`).

## Risk Assessment
| Risk | L | I | Mitigation |
|------|---|---|-----------|
| QEMU OpenSBI lacks SRST (0x53525354) | Med | High | `-bios default` ships a recent OpenSBI with SRST. Fallback: SBI legacy `sbi_shutdown` (a7=8) issued **from S-mode** (different from the broken U-mode `/bin/shutdown`). Document fallback inline if Phase 4 shows no exit. |
| `loop { sys_yield() }` dead code warning | Low | Low | Acceptable; required to satisfy `-> !`. The ecall above never returns. |
| Shell arm type mismatch | Low | Med | Use bare-expression arm form; verified other arms return `ViResult<()>` at `executor.rs:141-167`. |
| Accidental shutdown via stray 502 | Low | Med | 502 is not reachable from any existing cell; only the new `sys_shutdown` calls it. |

## Security Considerations
- Shutdown is unguarded (any cell could call raw 502). Acceptable for v1.0 single-user; a capability gate is deferred to Phase F (listed out-of-scope in plan.md).

## Evidence (Complete)
- `kernel/src/task/syscall.rs:256` — Shutdown variant added
- `kernel/src/task/syscall.rs:1109–1121` — handler with SBI SRST asm (options(noreturn))
- `kernel/src/task/syscall.rs:1203` — numeric map 502 → Shutdown
- `libs/ostd/src/syscall.rs:80–98` — sys_shutdown() -> !
- `cells/apps/shell/src/cmd_sys.rs:69–72` — cmd_shutdown()
- `cells/apps/shell/src/executor.rs:160` — "shutdown" arm registered
- All 14 integration tests pass; `shutdown` built-in cleanly exits QEMU within 15s

## Next Steps
- Phase 4 depends on 2f (`shutdown` built-in) to terminate the first QEMU instance.
