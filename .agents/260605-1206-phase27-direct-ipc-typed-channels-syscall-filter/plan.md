# Phase 27 — Direct IPC + Typed Channels + Syscall Filter

**Status**: ✅ COMPLETE (2026-06-07)  
**Priority**: P2  
**Target**: 2026-08-25 → **shipped ahead of schedule**  
**Effort**: ~4 weeks (implemented incrementally with Phase 26/30/TLS/Protocol-Hardening)  
**Created**: 2026-06-05

---

## Goal

Three improvements to ViCell's IPC and security model:

1. **Typed IPC Message Enums** — replace raw `[u8; 512]` byte buffers with postcard-serialized Rust enums defined in `libs/api/`. Type-safe request/response contracts between Cells.
2. **Syscall Allowlist** — embed a `u64` bitset in each Cell's ELF under `__ViCell_syscalls`; kernel reads it at spawn and enforces it at dispatch. No ecall for denied ops — immediate error return.
3. **Direct IPC vtable** — expose kernel service function pointers in a static read-only table; trusted Cells call them directly via pointer dereference (~3 cycles) instead of ecall (~100 cycles). Gated by a `TrustedHandle<T>` type token.

---

## Phases

| # | File | Status | Effort | Description |
|---|------|--------|--------|-------------|
| 1 | [phase-01-typed-ipc-enums.md](phase-01-typed-ipc-enums.md) | ✅ DONE | 4 days | postcard enums in libs/api replacing raw byte opcodes |
| 2 | [phase-02-syscall-allowlist.md](phase-02-syscall-allowlist.md) | ✅ DONE | 3 days | u64 bitset in ELF section, enforced at syscall dispatch |
| 3 | [phase-03-direct-ipc-vtable.md](phase-03-direct-ipc-vtable.md) | ✅ DONE | 5 days | Trusted Cell fast-path via kernel function pointer table |

**Execution order**: 1 → 2 → 3. Phases 1 and 2 are independent; Phase 3 builds on Phase 1 (needs typed messages to make the vtable API meaningful).

---

## ⚠️ Law 1 Gate

Phase 1 adds new types to `libs/api/` and Phase 2 adds `allowlist_bit()` to `ViSyscall`. Both require **2x user confirmation** before implementation.

Phase 3 adds `TrustedHandle<T>` to `libs/api/` — also Law 1.

---

## Current State (2026-06-05)

| Component | Current | Target |
|-----------|---------|--------|
| IPC wire format | Raw `(msg_ptr, msg_len)`, max 64 MiB, no type info | postcard-encoded enum in fixed 512-byte buffer |
| Syscall dispatch | Single match on ~36 opcodes; no per-Cell filter | u64 allowlist bitset, checked before dispatch |
| Trusted IPC | All Cells go through ecall → trap → dispatch | Direct fn-ptr call for trusted pairs (~3 cycles) |
| Message header | None (raw bytes) | Enum discriminant byte (postcard-encoded) |
| Lease/Grant | Basic Lease struct + BORROW_READ/WRITE (kernel copy) | SAS equivalent: blocked-lender invariant + direct slice ref |

---

## Key Design Decisions

### Typed IPC: postcard into existing [u8; 512]
Keep the existing `sys_send(target, ptr, len)` syscall ABI unchanged. `postcard::to_slice` writes the encoded enum into a stack-allocated `[u8; 512]`. Migration is additive — old byte-opcode services remain until each is updated.

### Syscall allowlist: separate bit-index from raw opcode
Raw opcodes (0-411, sparse) cannot be used directly as bit positions. A stable `allowlist_bit()` method on `ViSyscall` maps each opcode to bit 0-35. Raw syscalls 500-503 (BlkRead/Write) are handled by a separate check at the entry point.

**Critical**: the allowlist check MUST read the bitset and drop the SCHEDULER lock BEFORE calling `handle_syscall` — otherwise the two lock acquisitions (allowlist check + handle_syscall's internal locks) create contention.

### Direct IPC vtable: static function-pointer table
The kernel exports a `KERNEL_FAST_IPC_TABLE: [fn(&KernelFastIpcCtx) -> ViResult<()>; N]` in a read-only static. Trusted Cells (holding `TrustedHandle<T>` from Phase 26's cap module) call entries directly. No trap, no privilege switch, no SCHEDULER lock for read operations. Gate: only kernel code can write the table (at init time); no Cell writes it.

---

## Success Criteria

- [x] `VfsRequest` / `VfsResponse` / `NetRequest` / `NetResponse` / `InputRequest` postcard enums in `libs/api/src/ipc.rs`; all services use them
- [x] `cargo check --workspace` green (verified 2026-06-07 — 0 errors, warnings only)
- [x] `ViSyscall::allowlist_bit()` + `SyscallBitset` + `declare_syscalls!` macro in `libs/api/src/syscall.rs`
- [x] Kernel reads `__ViCell_syscalls` ELF section at spawn, stores bitset in TCB
- [x] Fast-IPC vtable: `kernel/src/fast_ipc.rs` (canonical instance), VFS registers handler at startup, shell uses fast path with ecall fallback. `clear_vfs_if_cell` wired into scheduler/exit path.
- [ ] Benchmark cycles (ecall vs vtable) — deferred: `sys_get_time` resolution (100ns) too coarse for 3-cycle measurement; needs `rdcycle` (G2 perf work)

## Evidence (2026-06-07)
- `libs/api/src/fast_ipc.rs` — `TrustedHandle<T>` ZST, `VfsCell`/`NetCell` markers
- `libs/ostd/src/fast_ipc.rs` — `register_vfs`, `call_vfs`, `clear_vfs_if_cell`, `vi_set/clear_fast_ipc_vfs_cell` no_mangle shims
- `kernel/src/fast_ipc.rs` — canonical AtomicPtr instance + `resolve_export` for loader dynamic linking
- `kernel/src/loader.rs:161,184` — `set_vfs_handler_cell` on VFS spawn
- `kernel/src/task.rs:211` + `task/scheduler.rs:512,634` — `clear_vfs_if_cell` on exit/fault
- `cells/services/vfs/src/main.rs:538-568` — `vfs_fast_handler` fn + `register_vfs` call at init
- `cells/apps/shell/src/cmd_fs.rs:307-335` — `read_file_vfs` with fast path + ecall fallback
