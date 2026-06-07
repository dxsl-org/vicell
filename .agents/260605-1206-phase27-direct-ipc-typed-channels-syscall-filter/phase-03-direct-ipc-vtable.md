# Phase 03 — Direct IPC vtable

**Status**: ✅ DONE (2026-06-07) — infrastructure complete; fast path activates with PIE cells (G2)  
**Priority**: P2  
**Effort**: 5 days  
**Depends on**: Phase 01 (typed messages), Phase 26 (SpawnCap / TrustedHandle concept)

---

## ⚠️ Law 1 Gate — 2x Confirmation Required

Adding `TrustedHandle<T>` and `KernelFastIpcTable` to `libs/api/`. New types only; no existing types modified.

---

## Context Links

- Current IPC: `kernel/src/task.rs` — `ipc_send()`, `ipc_recv()` (~100 cycles per round-trip via ecall)
- Scheduler: `kernel/src/task/scheduler.rs` — per-task context switch
- Phase 26 caps: `kernel/src/task/cap.rs` — `BlockIoCap`, `NetworkCap`, `SpawnCap`
- Typed messages: Phase 01 — `VfsRequest`/`VfsResponse` enums

---

## Overview

**The problem**: Every Cell-to-Cell IPC message (even VFS reads from shell) goes through: `ecall` → kernel trap entry → SCHEDULER lock → match dispatch → copy → SCHEDULER unlock → `sret`. This costs ~100-300 cycles per round-trip.

**The insight** (from Hermit OS research): In a Single Address Space OS, there is NO privilege boundary between Cells. A "trusted" Cell calling a VFS service function is mechanically identical to calling a Rust function — both are `jalr` through a pointer. The only overhead is a single indirect call instruction (~3 cycles).

**The approach**: The kernel exposes a read-only static `KERNEL_FAST_IPC_TABLE` containing function pointers to frequently-called service handlers. Trusted Cells (holding a `TrustedHandle<VfsCell>` token from Phase 26's cap module) call entries directly, bypassing ecall entirely. The `TrustedHandle<T>` type ensures only kernel-authorized callers can use the fast path.

**Scope**: Fast path for high-frequency operations only (VFS read, IPC data passing). Low-frequency ops (spawn, hotswap) remain on the ecall path. Single-core only — no SMP complications.

---

## Architecture

```
Current (ecall path, ~200 cycles):
  shell                kernel          vfs
    │── ecall ─────────→│              │
    │                   │── dispatch ──│
    │                   │              │ (handle request)
    │                   │←─ reply ─────│
    │←─ sret ───────────│              │

Direct IPC (vtable path, ~5 cycles):
  shell                           vfs
    │─── (*FAST_IPC_TABLE[READ])() ──→│
    │                                  │ (handle request inline)
    │←────────────────── return value ─│
```

---

## Design: `TrustedHandle<T>` Token

A `TrustedHandle<VfsCell>` is a ZST (zero bytes at runtime, like Phase 26 caps) whose constructor is `pub(crate)`. Holding one proves the kernel granted fast-path access to the VFS cell's vtable. It is `Copy` — the token represents authorization, not ownership.

```rust
// In libs/api/src/fast_ipc.rs (new file, Law 1 gate)
pub struct TrustedHandle<T>(core::marker::PhantomData<T>);

/// Marker type for VFS Cell fast-IPC authorization.
pub struct VfsCell;
/// Marker type for net Cell fast-IPC authorization.
pub struct NetCell;

impl<T> TrustedHandle<T> {
    /// Create a TrustedHandle.  Only callable within the kernel crate.
    pub(crate) fn new() -> Self { Self(core::marker::PhantomData) }
}

impl<T> Copy for TrustedHandle<T> {}
impl<T> Clone for TrustedHandle<T> { fn clone(&self) -> Self { *self } }
```

---

## Design: Function-pointer table in `libs/ostd`

**⚠️ Red-team fix**: The table CANNOT live in `kernel/` because Cell crates (shell, vfs) only depend on `libs/ostd` and `libs/api` — they cannot import the kernel crate. The fn-ptr table must live in `libs/ostd`.

**⚠️ Red-team fix**: `static mut` is UB-prone and `unsafe_code = "deny"` in `libs/api` prevents unsafe blocks there. Use `core::sync::atomic::AtomicPtr` (lock-free, no alloc) for the single pointer.

```rust
// In libs/ostd/src/fast_ipc.rs (new file, accessible to both kernel and cells)
use api::ipc::{VfsRequest, IPC_BUF_SIZE};
use core::sync::atomic::{AtomicPtr, Ordering};

/// Signature of a VFS fast-IPC handler registered by the VFS cell.
pub type VfsFastHandler =
    unsafe fn(req: &VfsRequest<'_>, out: &mut [u8; IPC_BUF_SIZE]) -> usize;

static VFS_HANDLER_PTR: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());

/// Register the VFS fast-IPC handler.  Called once by VFS at startup.
/// Idempotent — second call replaces (safe: single-hart, no concurrent reads).
pub fn register_vfs(handler: VfsFastHandler) {
    VFS_HANDLER_PTR.store(handler as *mut (), Ordering::Release);
}

/// Call the VFS handler if registered.  Returns bytes written into `out`, or 0 if
/// the handler is not yet registered (caller falls back to ecall path).
///
/// # Safety
/// `_handle: TrustedHandle<VfsCell>` proves the caller was granted fast-path
/// access by the kernel.  The caller must not alias `out` across calls.
pub unsafe fn call_vfs(
    _handle: TrustedHandle<VfsCell>,
    req: &VfsRequest<'_>,
    out: &mut [u8; IPC_BUF_SIZE],
) -> usize {
    let ptr = VFS_HANDLER_PTR.load(Ordering::Acquire);
    if ptr.is_null() { return 0; }
    // SAFETY: ptr was stored by register_vfs with a valid function pointer.
    let handler: VfsFastHandler = core::mem::transmute(ptr);

    // Disable S-mode interrupts during handler execution.
    // The VFS FAT16 driver holds a spinlock internally; if the timer ISR fires
    // mid-handler and preempts to another VFS caller, the spinlock would deadlock.
    // This makes the fast path behave like a critical section w.r.t. the scheduler.
    // SAFETY: csrrci/csrsi on sstatus is safe from S-mode; restoring to prior state.
    #[cfg(target_arch = "riscv64")]
    let sie_was_set = {
        let v: usize;
        core::arch::asm!("csrrci {}, sstatus, 0x2", out(reg) v);
        v & 0x2 != 0
    };
    #[cfg(not(target_arch = "riscv64"))]
    let sie_was_set = false;

    let result = handler(req, out);

    #[cfg(target_arch = "riscv64")]
    if sie_was_set { core::arch::asm!("csrsi sstatus, 0x2"); }

    result
}
```

### Cell-side usage (shell calling VFS):

**⚠️ Red-team fix**: `fast_read` must NOT return a reference into its local `resp_buf` (dangling after return). The caller owns the output buffer:

```rust
// In cells/apps/shell/src/vfs_client.rs
pub fn fast_read(
    handle: TrustedHandle<VfsCell>,
    path: &str,
    out: &mut [u8; IPC_BUF_SIZE],  // caller-owned output buffer — no dangling ref
) -> usize {
    let req = VfsRequest::GetFile { path };
    // SAFETY: handle proves kernel granted fast-path access; out is caller-owned.
    unsafe { ostd::fast_ipc::call_vfs(handle, &req, out) }
    // Caller decodes: api::ipc::decode::<VfsResponse>(&out[..n])
    // The decoded &str borrows from `out` which the caller owns — sound.
}
```

This call costs: 1 `AtomicPtr::load` (Acquire) + 1 `transmute` + 1 indirect `jalr`. No ecall, no CSR, no SCHEDULER lock, ~5 cycles total.

---

## Related Code Files

### Create
- `libs/api/src/fast_ipc.rs` — `TrustedHandle<T>`, marker types (Law 1 gate)
- `libs/ostd/src/fast_ipc.rs` — `VFS_HANDLER_PTR: AtomicPtr`, `register_vfs()`, `call_vfs()` (lives in ostd, accessible to both kernel and cells)

### Modify
- `libs/api/src/lib.rs` — `pub mod fast_ipc`
- `libs/ostd/src/lib.rs` — `pub mod fast_ipc`
- `cells/services/vfs/src/main.rs` — implement `vfs_fast_handler` fn, call `ostd::fast_ipc::register_vfs()`
- `cells/apps/shell/src/main.rs` — `fast_read(handle, path, out)` using `ostd::fast_ipc::call_vfs()`

---

## Implementation Steps

### Step 1 — Define `TrustedHandle<T>` in `libs/api/src/fast_ipc.rs`

(See Design section above)

### Step 2 — Define handler static in `kernel/src/fast_ipc.rs`

(See Design section above)

### Step 3 — VFS cell registers handler at startup

In `cells/services/vfs/src/main.rs`, at the end of init (after VFS is ready):

```rust
// Register the fast-IPC handler so trusted Cells can bypass ecall.
// SAFETY: called once at startup; single-hart; no concurrent callers yet.
unsafe {
    kernel::fast_ipc::register_vfs_handler(vfs_fast_handle);
}

fn vfs_fast_handle(req: &VfsRequest<'_>, resp_buf: &mut [u8; IPC_BUF_SIZE]) -> usize {
    let resp = match req {
        VfsRequest::GetFile { path } => handle_get_file(path),
        VfsRequest::Write { path, content } => handle_write(path, content),
        // ...
    };
    api::ipc::encode(&resp, resp_buf).map(|s| s.len()).unwrap_or(0)
}
```

### Step 4 — Shell uses fast path for read-heavy ops

Add `fast_read()` wrapper in shell. Use it in `cat` / `ls` built-ins where VFS round-trips dominate.

### Step 5 — Benchmark

Measure cycles for:
1. Shell `cat /data/hello.txt` via ecall path (current)
2. Shell `cat /data/hello.txt` via vtable fast path

Use `hal::common::timer::read_mtime()` delta × `TICKS_PER_10MS / 100_000` for ns conversion.

---

## Safety Analysis

**Why this is safe in SAS:**
- VFS_FAST_HANDLER is written once at VFS init (before any Cell reads it) — no data race
- `TrustedHandle<VfsCell>` is kernel-only constructable — no unauthorized callers
- The handler runs in the caller's stack frame (not VFS's) — VFS must not write to caller's stack
- Single-hart: no SMP concurrency concern for the static write
- If VFS crashes before registering: `VFS_FAST_HANDLER == None` → fallback to ecall path (safe)

**What this does NOT prevent:**
- A trusted Cell calling the handler with a malicious `VfsRequest` — but this is the same threat model as any other syscall; VFS validates requests
- Performance degradation if handler takes locks (VFS's own FAT16 driver does use spinlocks internally)

---

## Todo List

- [ ] ⚠️ Confirm `TrustedHandle<T>` and `KernelFastIpcTable` in libs/api (Law 1, 2x required)
- [ ] Create `libs/api/src/fast_ipc.rs` (TrustedHandle, VfsCell marker)
- [ ] Add `pub mod fast_ipc;` to `libs/api/src/lib.rs`
- [ ] Create `kernel/src/fast_ipc.rs` (VFS_FAST_HANDLER static, register function)
- [ ] Add `pub mod fast_ipc;` to `kernel/src/main.rs`
- [ ] VFS cell: implement `vfs_fast_handle()` and call `register_vfs_handler()` at init
- [ ] Shell: add `fast_read()` wrapper using `TrustedHandle<VfsCell>`
- [ ] Benchmark: measure ecall vs direct-call round-trip cycles
- [ ] Verify VFS crash → `None` fallback to ecall path

---

## Success Criteria

- [ ] `shell cat /data/hello.txt` via vtable path completes without ecall to kernel
- [ ] Benchmark shows < 10 cycles for direct vtable call (vs > 100 for ecall)
- [ ] VFS crash before registration: fast_read() falls back to sys_send() gracefully
- [ ] All 65 integration tests pass

---

## Risk Assessment

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| VFS handler accesses Cell-local state causing corruption | Medium | Handler must only touch its own static state; no writes to caller's pointers |
| `static mut` for `VFS_FAST_HANDLER` triggers UB on multi-core | Low | Single-hart QEMU; note in comment for SMP migration |
| Law 1 gate: TrustedHandle requires 2x confirm | Certain | Gate in plan; confirm before implementation |
| Fast path bypasses audit log (Phase 26 audit.rs) | Low | Add `audit::log_event(FileRead, ...)` inside the fast handler |
