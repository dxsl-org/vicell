//! Per-hart local state, accessed in O(1) via the `tp` (thread-pointer) CSR.
//!
//! Each hart keeps `tp = &HART_LOCALS[hart_id]` at all times while running
//! kernel code.  The trap entry restores the kernel `tp` on U→S transitions
//! so all kernel code (syscall handler, allocator, scheduler) always sees the
//! correct HartLocal.  Cells run with the value stored in
//! `ViHartLocal::kernel_tp_for_cells`, which is independent of HartLocal.
//!
//! Phase 02 replaces the single global `CURRENT_CELL_ID` with a per-hart
//! `current_cell_id` field inside `ViHartLocal`.  Phase 03 adds per-hart
//! ready queues and the work-stealing scheduler.

use crate::task::smp::MAX_HARTS;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Per-hart local state.
///
/// LAYOUT IS ABI — the trap.S reads `kernel_tp_for_cells` by hardcoded offset
/// and the field order is FIXED.  Add new fields AFTER existing ones.
/// `#[repr(C)]` ensures Rust does not reorder or pad unexpectedly.
#[repr(C)]
pub struct ViHartLocal {
    /// This hart's id (0 = boot hart).
    pub hart_id: usize,              // offset 0
    /// Cell ID currently running on this hart.  0 = kernel (no quota limit).
    pub current_cell_id: AtomicUsize, // offset 8  (AtomicUsize is transparent over usize)
    /// Value of `gp` captured at `install()` time — handed to new cells.
    pub kernel_gp: usize,            // offset 16
    /// Value of `tp` that cells inherit on context switch.  Currently 0 (cells
    /// have no TLS); Phase 05 may give each cell a private tp.
    pub kernel_tp_for_cells: usize,  // offset 24
}

/// Static array of per-hart local state, one entry per supported hart.
/// Accessed without any lock: hart N only writes HART_LOCALS[N] from N.
/// SAFETY: interior mutability via AtomicUsize; the `usize` fields are only
/// written during `install()` before the hart handles any interrupt.
pub static HART_LOCALS: [ViHartLocal; MAX_HARTS] = {
    const ZERO: ViHartLocal = ViHartLocal {
        hart_id: 0,
        current_cell_id: AtomicUsize::new(0),
        kernel_gp: 0,
        kernel_tp_for_cells: 0,
    };
    [ZERO; MAX_HARTS]
};

/// Pointer to the calling hart's `ViHartLocal`, stored as a plain `usize`.
///
/// `trap.S` loads `tp` from this address on every U→S trap so kernel code
/// always runs with a valid hart-local pointer.  Exposed `#[no_mangle]` so
/// the assembler can reference it by name without mangling.
///
/// Phase 03 upgrades this to the `sscratch = &HartLocal` protocol for full
/// SMP correctness.  For Phase 02 (single active hart), the single-entry
/// array gives the correct result.
#[no_mangle]
pub static HART_LOCAL_TP_ADDR: AtomicUsize = AtomicUsize::new(0);

/// Initialize the calling hart's `ViHartLocal` and write `tp` to point at it.
///
/// Call BEFORE enabling the scheduler or handling any interrupt on this hart.
/// Hart 0 calls this as the first action of `task::init()`.
/// Secondary harts call this from `smp_hart_entry`, after installing stvec.
pub fn install(hart_id: usize) {
    assert!(hart_id < MAX_HARTS, "hart_id {} >= MAX_HARTS {}", hart_id, MAX_HARTS);

    // Capture the current gp and tp so we can hand them to cells unchanged.
    let (gp, tp) = crate::hal::arch::get_gp_tp();

    // SAFETY: hart_id < MAX_HARTS; we only write HART_LOCALS[hart_id] from
    // the hart with that id — no concurrent writers for this index.
    let hl = &HART_LOCALS[hart_id];

    // Write hart_id once (not atomic — no other hart touches this slot yet).
    // SAFETY: single writer; written before any reader could observe this hart.
    unsafe {
        let ptr = hl as *const ViHartLocal as *mut ViHartLocal;
        core::ptr::addr_of_mut!((*ptr).hart_id).write(hart_id);
        core::ptr::addr_of_mut!((*ptr).kernel_gp).write(gp);
        core::ptr::addr_of_mut!((*ptr).kernel_tp_for_cells).write(tp);
    }
    hl.current_cell_id.store(0, Ordering::Relaxed);

    // Point HART_LOCAL_TP_ADDR at this hart's slot so trap.S can restore tp.
    // For single-hart, this is always HART_LOCALS[0].
    // Phase 03 (SMP) will update to the sscratch = &HartLocal protocol instead.
    let hl_addr = hl as *const ViHartLocal as usize;
    HART_LOCAL_TP_ADDR.store(hl_addr, Ordering::Release);

    // Write tp CSR to point at this hart's HartLocal.
    // SAFETY: tp is a callee-save GPR used here as a kernel-internal pointer;
    // cells receive `kernel_tp_for_cells` (not this pointer) on context switch.
    unsafe { write_tp(hl_addr) };
}

/// Return a reference to the calling hart's `ViHartLocal`.
///
/// On RISC-V reads the `tp` CSR. On other architectures (x86_64 single-hart
/// bring-up) returns HART_LOCALS[0] directly.
///
/// # Safety
/// On RISC-V: `tp` must point to a valid `ViHartLocal` (guaranteed after `install()`).
#[inline(always)]
pub unsafe fn current_hart() -> &'static ViHartLocal {
    #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
    {
        let tp: usize;
        core::arch::asm!("mv {}, tp", out(reg) tp, options(nomem, nostack, preserves_flags));
        &*(tp as *const ViHartLocal)
    }
    #[cfg(not(any(target_arch = "riscv64", target_arch = "riscv32")))]
    { &HART_LOCALS[0] }
}

/// Return the calling hart's id.
///
/// Returns 0 if called before `install()` (safe — hart 0 is hart 0).
#[inline(always)]
pub fn current_hart_id() -> usize {
    #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
    {
        let tp: usize;
        unsafe {
            core::arch::asm!("mv {}, tp", out(reg) tp, options(nomem, nostack, preserves_flags));
        }
        if tp == 0 { return 0; }
        unsafe { (*(tp as *const ViHartLocal)).hart_id }
    }
    #[cfg(not(any(target_arch = "riscv64", target_arch = "riscv32")))]
    { 0 }
}

/// Cell ID currently running on this hart (0 = kernel, no quota).
///
/// On RISC-V reads `tp` CSR. On other architectures returns HART_LOCALS[0].current_cell_id.
#[inline(always)]
pub fn current_cell_id() -> usize {
    #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
    {
        let tp: usize;
        unsafe {
            core::arch::asm!("mv {}, tp", out(reg) tp, options(nomem, nostack, preserves_flags));
        }
        if tp == 0 { return 0; }
        unsafe { (*(tp as *const ViHartLocal)).current_cell_id.load(Ordering::Relaxed) }
    }
    #[cfg(not(any(target_arch = "riscv64", target_arch = "riscv32")))]
    { HART_LOCALS[0].current_cell_id.load(Ordering::Relaxed) }
}

/// Update the cell-id attribution for the calling hart.
#[inline(always)]
pub fn set_current_cell_id(id: usize) {
    #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
    {
        let tp: usize;
        unsafe {
            core::arch::asm!("mv {}, tp", out(reg) tp, options(nomem, nostack, preserves_flags));
        }
        if tp == 0 { return; }
        unsafe { (*(tp as *const ViHartLocal)).current_cell_id.store(id, Ordering::Relaxed) };
    }
    #[cfg(not(any(target_arch = "riscv64", target_arch = "riscv32")))]
    { HART_LOCALS[0].current_cell_id.store(id, Ordering::Relaxed); }
}

/// Write the `tp` register.
///
/// # Safety
/// Caller is responsible for ensuring `val` is a valid `ViHartLocal` pointer
/// (or 0 for the pre-install sentinel).  Must run with interrupts disabled or
/// from boot context where no concurrent trap can misread a partial write.
#[inline(always)]
#[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
pub unsafe fn write_tp(val: usize) {
    // SAFETY: writing tp CSR is always safe from S-mode; the value is either
    // a valid HART_LOCALS pointer or 0 (pre-install). Caller ensures context.
    core::arch::asm!("mv tp, {}", in(reg) val, options(nomem, nostack, preserves_flags));
}

#[cfg(not(any(target_arch = "riscv64", target_arch = "riscv32")))]
pub unsafe fn write_tp(_val: usize) {}
