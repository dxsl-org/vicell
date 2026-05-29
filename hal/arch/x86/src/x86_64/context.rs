//! x86_64 CPU context (callee-saved registers + RSP for cooperative switch).
use core::arch::asm;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuContext {
    pub r15: u64, pub r14: u64, pub r13: u64, pub r12: u64,
    pub rbx: u64, pub rbp: u64, pub rsp: u64, pub rip: u64,
}

/// Cooperative context switch.
///
/// # Safety
/// Both pointers must point to valid, aligned `CpuContext` structs.
pub unsafe fn switch(old: *mut CpuContext, new: *const CpuContext) {
    // SAFETY: caller guarantees valid, aligned CpuContext pointers.
    unsafe {
        asm!(
            "mov [{o}+0*8], r15",  "mov [{o}+1*8], r14",
            "mov [{o}+2*8], r13",  "mov [{o}+3*8], r12",
            "mov [{o}+4*8], rbx",  "mov [{o}+5*8], rbp",
            "mov [{o}+6*8], rsp",
            "lea rax, [rip+99f]",   "mov [{o}+7*8], rax",
            "mov r15, [{n}+0*8]",  "mov r14, [{n}+1*8]",
            "mov r13, [{n}+2*8]",  "mov r12, [{n}+3*8]",
            "mov rbx, [{n}+4*8]",  "mov rbp, [{n}+5*8]",
            "mov rsp, [{n}+6*8]",
            "jmp [{n}+7*8]",
            "99:",
            o = in(reg) old, n = in(reg) new, out("rax") _,
            // Intel syntax — no att_syntax option needed
        );
    }
}
