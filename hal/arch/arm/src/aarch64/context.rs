//! AArch64 CPU context for cooperative context switching.

use core::arch::global_asm;

/// Saved CPU context for a kernel-side task.
///
/// Stores the callee-saved registers (x19..x30) plus the stack pointer and
/// system registers that must survive a context switch in EL1.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct CpuContext {
    // Callee-saved per AArch64 ABI.
    pub x19: u64,
    pub x20: u64,
    pub x21: u64,
    pub x22: u64,
    pub x23: u64,
    pub x24: u64,
    pub x25: u64,
    pub x26: u64,
    pub x27: u64,
    pub x28: u64,
    pub x29: u64, // frame pointer
    pub x30: u64, // link register (return address)
    pub sp:  u64, // stack pointer
    pub elr_el1:  u64,
    pub spsr_el1: u64,
}

/// Perform a context switch from `old` to `new`.
///
/// # Safety
/// Both pointers must point to valid, aligned `CpuContext` structs that will
/// not be deallocated before the context switch completes.
pub unsafe fn switch(old: *mut CpuContext, new: *const CpuContext) {
    // SAFETY: delegated to the assembly stub below.
    unsafe { __switch(old, new); }
}

extern "C" {
    fn __switch(old: *mut CpuContext, new: *const CpuContext);
}

global_asm!(
    r#"
    .section .text
    .global __switch
    .balign 4
__switch:
    // x0 = old CpuContext*, x1 = new CpuContext*
    // Save callee-saved + sp + link register.
    stp  x19, x20, [x0, #0]
    stp  x21, x22, [x0, #16]
    stp  x23, x24, [x0, #32]
    stp  x25, x26, [x0, #48]
    stp  x27, x28, [x0, #64]
    stp  x29, x30, [x0, #80]
    mov  x9,  sp
    str  x9,       [x0, #96]
    mrs  x9,  elr_el1
    mrs  x10, spsr_el1
    stp  x9,  x10, [x0, #104]

    // Restore new context.
    ldp  x19, x20, [x1, #0]
    ldp  x21, x22, [x1, #16]
    ldp  x23, x24, [x1, #32]
    ldp  x25, x26, [x1, #48]
    ldp  x27, x28, [x1, #64]
    ldp  x29, x30, [x1, #80]
    ldr  x9,       [x1, #96]
    mov  sp,  x9
    ldp  x9,  x10, [x1, #104]
    msr  elr_el1,  x9
    msr  spsr_el1, x10

    ret
    "#
);
