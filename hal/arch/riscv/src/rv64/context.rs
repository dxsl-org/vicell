#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct Context {
    pub ra: usize,
    pub sp: usize,
    pub s0: usize,
    pub s1: usize,
    pub s2: usize,
    pub s3: usize,
    pub s4: usize,
    pub s5: usize,
    pub s6: usize,
    pub s7: usize,
    pub s8: usize,
    pub s9: usize,
    pub s10: usize,
    pub s11: usize,
    pub sepc: usize,
    pub sstatus: usize,
    pub gp: usize,
    pub tp: usize,
    pub sscratch: usize,
}

// Assembly implementation handled by build.rs

impl Context {
    /// Perform a context switch.
    ///
    /// # Safety
    /// This function performs a raw context switch and must be called with valid pointers.
    #[inline(always)]
    pub unsafe fn switch(old: *mut Context, new: *const Context) {
        // External assembly implementation
        extern "C" {
            fn __switch(old: *mut Context, new: *const Context);
        }
        __switch(old, new);
    }
}

pub fn get_gp_tp() -> (usize, usize) {
    let gp: usize;
    let tp: usize;
    unsafe {
        #[cfg(target_arch = "riscv64")]
        {
            core::arch::asm!("mv {0}, gp", out(reg) gp);
            core::arch::asm!("mv {0}, tp", out(reg) tp);
        }
        #[cfg(not(target_arch = "riscv64"))]
        {
            gp = 0;
            tp = 0;
        }
    }
    (gp, tp)
}

/// Read the current value of the `tp` (thread-pointer) register.
///
/// # Safety
/// Reading tp is always safe from S-mode; no side effects.
#[cfg(target_arch = "riscv64")]
pub unsafe fn read_tp() -> usize {
    let tp: usize;
    core::arch::asm!("mv {0}, tp", out(reg) tp, options(nomem, nostack, preserves_flags));
    tp
}

/// Write a new value to the `tp` (thread-pointer) register.
///
/// # Safety
/// Caller must ensure `val` is either 0 or a pointer to a valid `ViHartLocal`.
/// Should only be called from boot context or `hart_local::install()` with
/// interrupts disabled so no trap fires with a half-written tp.
#[cfg(target_arch = "riscv64")]
pub unsafe fn write_tp(val: usize) {
    // SAFETY: writing tp from S-mode is always permitted; caller ensures value
    // is a valid ViHartLocal pointer.
    core::arch::asm!("mv tp, {0}", in(reg) val, options(nomem, nostack, preserves_flags));
}
