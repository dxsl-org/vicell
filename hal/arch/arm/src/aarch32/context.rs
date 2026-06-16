//! AArch32 cooperative context switch — AAPCS callee-saved frame.
//!
//! Saves r4-r11, sp (r13), lr (r14 = resume address), and cpsr.
//! The `__switch_arm32` function is in ARM state (not Thumb) so the
//! kernel linker script must place it in a code section without thumb interwork.

use core::arch::global_asm;

/// Callee-saved register frame for ARMv7-A cooperative task switch.
///
/// Offsets (must match `__switch_arm32` asm):
///   0..28 = r4..r11 (8×4),  32 = sp,  36 = lr,  40 = cpsr
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Arm32Context {
    pub r4:   u32, // offset  0
    pub r5:   u32, // offset  4
    pub r6:   u32, // offset  8
    pub r7:   u32, // offset 12
    pub r8:   u32, // offset 16
    pub r9:   u32, // offset 20
    pub r10:  u32, // offset 24
    pub r11:  u32, // offset 28
    pub sp:   u32, // offset 32 — kernel stack pointer
    pub lr:   u32, // offset 36 — resume address (caller's LR)
    pub cpsr: u32, // offset 40 — CPSR flags
}

// ── Context-switch routine ───────────────────────────────────────────────────
// ARM AAPCS: r0 = old_ptr, r1 = new_ptr
global_asm!(
    ".global __switch_arm32",
    ".arm",                             // force ARM (not Thumb) encoding
    "__switch_arm32:",
    // Save r4-r11 at *r0, post-increment r0 by 32.
    "stmia r0!, {{r4-r11}}",
    // Save sp at old->sp (r0 now points past r11, i.e. offset 32).
    "str sp, [r0]",
    // Save lr at old->lr (offset 36).
    "str lr, [r0, #4]",
    // Save cpsr at old->cpsr (offset 40).
    "mrs r2, cpsr",
    "str r2, [r0, #8]",
    // Restore r4-r11 from *r1, post-increment r1 by 32.
    "ldmia r1!, {{r4-r11}}",
    // Restore sp from new->sp.
    "ldr sp, [r1]",
    // Restore lr from new->lr.
    "ldr lr, [r1, #4]",
    // Restore cpsr from new->cpsr.
    "ldr r2, [r1, #8]",
    "msr cpsr_cxsf, r2",
    // Return into the new task (lr = resume address).
    "bx lr",
);

extern "C" {
    fn __switch_arm32(old: *mut Arm32Context, new: *const Arm32Context);
}

impl Arm32Context {
    /// Cooperative context switch from `old` to `new`.
    ///
    /// # Safety
    /// Both pointers must be valid, non-null, 4-byte-aligned, and non-aliasing.
    pub unsafe fn switch(old: *mut Arm32Context, new: *const Arm32Context) {
        // SAFETY: invariants enforced by the caller; __switch_arm32 is pure asm.
        __switch_arm32(old, new);
    }
}

/// Free-function alias for use in aarch32.rs `Arch::switch_context`.
pub unsafe fn switch(old: *mut Arm32Context, new: *const Arm32Context) {
    Arm32Context::switch(old, new);
}
