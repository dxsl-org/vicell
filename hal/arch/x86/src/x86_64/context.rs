//! x86_64 CPU context (callee-saved registers + RSP for cooperative switch).
use core::arch::asm;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuContext {
    pub r15: u64, pub r14: u64, pub r13: u64, pub r12: u64,
    pub rbx: u64, pub rbp: u64,
    /// Kernel stack pointer (mapped to RSP; offset 6*8 in the switch asm).
    pub sp: u64,
    /// Resume instruction pointer (mapped to RIP via `jmp`; offset 7*8).
    pub rip: u64,
    /// Fixed syscall-entry RSP = kstack_top − TRAP_FRAME_SIZE.
    /// Set once at spawn; NEVER updated by cooperative switches.
    /// Used by `set_kernel_stack` so CPU_LOCAL.kernel_rsp always points to
    /// the top of a fresh syscall frame, not the deep cooperative-switch RSP
    /// that shrinks every blocking cycle.
    pub kernel_trap_sp: u64,
}

impl CpuContext {
    /// Cooperative context switch — associated-function form used by the kernel.
    ///
    /// # Safety
    /// Both pointers must point to valid, aligned `CpuContext` structs.
    #[inline(always)]
    pub unsafe fn switch(old: *mut CpuContext, new: *const CpuContext) {
        // SAFETY: invariant upheld by caller.
        unsafe { switch(old, new) }
    }
}

/// Cooperative context switch.
///
/// # Safety
/// Both pointers must point to valid, aligned `CpuContext` structs.
pub unsafe fn switch(old: *mut CpuContext, new: *const CpuContext) {
    // SAFETY: caller guarantees valid, aligned CpuContext pointers.
    //
    // Register discipline: pin `old` → rdi and `new` → rsi (SysV argument
    // registers).  Neither is ever written by the asm body — only their
    // *pointed-to* memory is touched — so both survive intact through the
    // jmp.  Without explicit pins, LLVM may assign `new` to r15/r14/r13/r12/
    // rbx/rbp, which the body overwrites, corrupting the pointer before the
    // final `jmp [rsi+7*8]` and causing a triple-fault (#PF at ~address 0).
    unsafe {
        asm!(
            "mov [rdi+0*8], r15",  "mov [rdi+1*8], r14",
            "mov [rdi+2*8], r13",  "mov [rdi+3*8], r12",
            "mov [rdi+4*8], rbx",  "mov [rdi+5*8], rbp",
            "mov [rdi+6*8], rsp",
            "lea rax, [rip+99f]",   "mov [rdi+7*8], rax",
            "mov r15, [rsi+0*8]",  "mov r14, [rsi+1*8]",
            "mov r13, [rsi+2*8]",  "mov r12, [rsi+3*8]",
            "mov rbx, [rsi+4*8]",  "mov rbp, [rsi+5*8]",
            "mov rsp, [rsi+6*8]",
            "jmp [rsi+7*8]",
            "99:",
            in("rdi") old, in("rsi") new,
            out("rax") _,
        );
    }
}
