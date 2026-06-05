//! Fast-IPC handler table for direct Cell-to-service calls.
//!
//! Bypasses the ecall trap for high-frequency VFS operations (~3 cycles vs
//! ~100 cycles for a full syscall round-trip in a Single Address Space OS).
//!
//! ## Safety invariant
//! The handler function pointer is written once at VFS startup (before any
//! Cell requests it) and never changed thereafter.  On single-hart QEMU,
//! there is no concurrent modification risk.  `Ordering::Release` on write
//! and `Ordering::Acquire` on read ensure the handler body is visible to
//! the caller after the pointer is stored.
//!
//! ## Preemption
//! `call_vfs` disables S-mode interrupts for the duration of the handler call.
//! The VFS FAT16 driver holds a spinlock; if the timer ISR preempted the
//! handler mid-spinlock and switched to another Cell that also called VFS,
//! the spinlock would deadlock.  Interrupt-disable makes the fast-path call
//! behave as an atomic critical section w.r.t. the scheduler.

use api::fast_ipc::{TrustedHandle, VfsCell};
use api::ipc::{VfsRequest, IPC_BUF_SIZE};
use core::sync::atomic::{AtomicPtr, Ordering};

/// Signature of a registered VFS fast-IPC handler.
///
/// Reads the request and writes the response into `out`.
/// Returns the number of bytes written into `out`.
pub type VfsFastHandler =
    unsafe fn(req: &VfsRequest<'_>, out: &mut [u8; IPC_BUF_SIZE]) -> usize;

static VFS_HANDLER_PTR: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());

/// Register the VFS fast-IPC handler.
///
/// Called once by the VFS service at startup.  The pointer is published with
/// `Release` ordering so subsequent `Acquire` loads in `call_vfs` observe the
/// complete handler function.
pub fn register_vfs(handler: VfsFastHandler) {
    VFS_HANDLER_PTR.store(
        // SAFETY: transmuting fn pointer to *mut () for atomic storage;
        // the pointer is recovered with the same type in call_vfs.
        unsafe { core::mem::transmute(handler) },
        Ordering::Release,
    );
}

/// Call the registered VFS handler directly, bypassing the ecall trap.
///
/// Returns the number of bytes written into `out`, or 0 if no handler is
/// registered (caller should fall back to the `sys_send` / `sys_recv` path).
///
/// # Safety
/// The caller must own `out` exclusively for the duration of this call.
/// `_handle: TrustedHandle<VfsCell>` documents that the caller has been
/// granted fast-path access; it does not enforce this at runtime.
pub unsafe fn call_vfs(
    _handle: TrustedHandle<VfsCell>,
    req: &VfsRequest<'_>,
    out: &mut [u8; IPC_BUF_SIZE],
) -> usize {
    let ptr = VFS_HANDLER_PTR.load(Ordering::Acquire);
    if ptr.is_null() {
        return 0; // VFS not yet registered — caller falls back to ecall path
    }
    // SAFETY: ptr was stored by register_vfs from a valid VfsFastHandler.
    let handler: VfsFastHandler = core::mem::transmute(ptr);

    // Disable S-mode interrupts for the handler's duration.
    // VFS's FAT16 driver holds a spinlock; timer-preemption mid-handler could
    // switch to another VFS caller and deadlock on the same spinlock.
    #[cfg(target_arch = "riscv64")]
    let sie_was_set = {
        let v: usize;
        // SAFETY: csrrci reads and clears SIE (bit 1) atomically from S-mode.
        core::arch::asm!("csrrci {}, sstatus, 0x2", out(reg) v);
        v & 0x2 != 0
    };
    #[cfg(not(target_arch = "riscv64"))]
    let sie_was_set = false;

    let result = handler(req, out);

    // Restore SIE to its prior state.
    // SAFETY: restoring to the value we saved above; no invariant violated.
    #[cfg(target_arch = "riscv64")]
    if sie_was_set {
        core::arch::asm!("csrsi sstatus, 0x2");
    }

    result
}
