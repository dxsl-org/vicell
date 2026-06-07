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
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

/// Signature of a registered VFS fast-IPC handler.
///
/// Reads the request and writes the response into `out`.
/// Returns the number of bytes written into `out`.
pub type VfsFastHandler =
    unsafe fn(req: &VfsRequest<'_>, out: &mut [u8; IPC_BUF_SIZE]) -> usize;

static VFS_HANDLER_PTR: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());
/// Raw CellId of the cell that registered the VFS handler; 0 = unregistered.
/// Used to clear the handler pointer when the VFS cell crashes.
static VFS_HANDLER_CELL: AtomicUsize = AtomicUsize::new(0);

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

/// Record which cell (by CellId raw value) owns the registered VFS handler.
///
/// Call this immediately after `register_vfs` so the kernel can clear the
/// handler pointer if the owning cell crashes.
pub fn set_vfs_handler_cell(cell_id_raw: usize) {
    VFS_HANDLER_CELL.store(cell_id_raw, Ordering::Relaxed);
}

/// Clear the VFS handler pointer if `cell_id_raw` is the registered owner.
///
/// Called by the kernel fault-isolation path when a cell is terminated, so
/// that a future `call_vfs` does not jump into stale/replaced code.
pub fn clear_vfs_if_cell(cell_id_raw: usize) {
    if VFS_HANDLER_CELL.load(Ordering::Relaxed) == cell_id_raw && cell_id_raw != 0 {
        VFS_HANDLER_PTR.store(core::ptr::null_mut(), Ordering::Release);
        VFS_HANDLER_CELL.store(0, Ordering::Relaxed);
    }
}

/// `extern "Rust"` shim so the kernel (which does not depend on ostd) can
/// call `set_vfs_handler_cell` via link-time symbol resolution.
#[no_mangle]
pub extern "Rust" fn vi_set_fast_ipc_vfs_cell(cell_id: usize) {
    set_vfs_handler_cell(cell_id);
}

/// `extern "Rust"` shim so the kernel can call `clear_vfs_if_cell`.
#[no_mangle]
pub extern "Rust" fn vi_clear_fast_ipc_vfs_cell(cell_id: usize) {
    clear_vfs_if_cell(cell_id);
}

/// RAII guard: disables S-mode interrupt enable (SIE) on construction, restores on drop.
///
/// Ensures SIE is always restored even if the fast-IPC handler panics (drop glue
/// runs before the panic handler, so the restore is guaranteed).
struct SieGuard(bool);

impl SieGuard {
    /// Disable SIE and return a guard.  Noop on non-riscv64.
    ///
    /// # Safety
    /// Must be called from S-mode.
    #[inline]
    unsafe fn disable() -> Self {
        #[cfg(target_arch = "riscv64")]
        {
            let v: usize;
            // SAFETY: csrrci reads-and-clears sstatus.SIE (bit 1) atomically.
            core::arch::asm!("csrrci {}, sstatus, 0x2", out(reg) v);
            Self(v & 0x2 != 0)
        }
        #[cfg(not(target_arch = "riscv64"))]
        Self(false)
    }
}

impl Drop for SieGuard {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: restoring SIE to the state saved in disable(); S-mode only.
            #[cfg(target_arch = "riscv64")]
            unsafe { core::arch::asm!("csrsi sstatus, 0x2"); }
        }
    }
}

/// Call the registered VFS handler directly, bypassing the ecall trap.
///
/// Returns the number of bytes written into `out`, or 0 if no handler is
/// registered (caller should fall back to the `sys_send` / `sys_recv` path).
///
/// # Note (PIE limitation)
/// For non-PIE cells (current default), each cell ELF has its own copy of
/// `VFS_HANDLER_PTR` (statically linked from `libs/ostd`).  VFS writes to its
/// copy; client cells read from theirs (null) → always fallback to ecall.
/// The fast path activates once cells are compiled as PIE and the kernel loader
/// patches JUMP_SLOT relocations to `kernel::fast_ipc::call_vfs` (G2 work).
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

    // Disable S-mode interrupts for the handler's duration. VFS's FAT32 driver
    // holds a spinlock; timer-preemption mid-handler to another VFS caller would
    // deadlock on it. SieGuard restores on drop — safe even on handler panic.
    // SAFETY: called from S-mode; SieGuard::disable is safe here.
    let _sie = SieGuard::disable();

    handler(req, out)
}
