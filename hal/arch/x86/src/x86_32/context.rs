//! x86_32 cooperative context switch — callee-saved register frame.
//!
//! CpuContext32 stores the minimum state for a cooperative switch between
//! kernel-mode tasks (System V i386 ABI callee-saved set).

use core::arch::global_asm;

/// Callee-saved register frame for 32-bit x86 cooperative task switch.
///
/// Offsets (must match `__switch_x86_32` asm):
///   0  = ebx,  4  = esi,  8  = edi,  12 = ebp
///   16 = sp (ESP),  20 = eip   (resume address)
///
/// The stack pointer field is named `sp` (not `esp`) to match the common
/// `context.sp` API used by the scheduler across all architectures.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuContext32 {
    pub ebx: u32,
    pub esi: u32,
    pub edi: u32,
    pub ebp: u32,
    pub sp:  u32, // saved stack pointer (ESP)
    pub eip: u32, // resume address (return address from switch call)
}

// ── Context-switch routine ───────────────────────────────────────────────────
global_asm!(
    ".global __switch_x86_32",
    "__switch_x86_32:",
    // cdecl: [esp+4] = old_ptr, [esp+8] = new_ptr
    "mov eax, [esp+4]",     // eax = old CpuContext32*
    "mov ecx, [esp+8]",     // ecx = new CpuContext32*
    // Save old callee-saved registers.
    "mov [eax+0],  ebx",
    "mov [eax+4],  esi",
    "mov [eax+8],  edi",
    "mov [eax+12], ebp",
    // Save EIP (return address sitting at [esp]).
    "mov edx, [esp]",
    "mov [eax+20], edx",
    // Save ESP (pointing at the return address; caller sees it post-ret).
    "mov [eax+16], esp",
    // Restore new callee-saved registers.
    "mov ebx, [ecx+0]",
    "mov esi, [ecx+4]",
    "mov edi, [ecx+8]",
    "mov ebp, [ecx+12]",
    // Switch stack and jump to the new task's resume address.
    "mov esp, [ecx+16]",
    "jmp dword ptr [ecx+20]",
);

extern "C" {
    fn __switch_x86_32(old: *mut CpuContext32, new: *const CpuContext32);
}

impl CpuContext32 {
    /// Cooperative context switch from `old` to `new`.
    ///
    /// # Safety
    /// Both pointers must be valid, non-null, 4-byte-aligned, and non-aliasing.
    pub unsafe fn switch(old: *mut CpuContext32, new: *const CpuContext32) {
        // SAFETY: invariants enforced by caller; __switch_x86_32 is pure asm.
        __switch_x86_32(old, new);
    }
}

/// Free-function alias for use in x86_32.rs `Arch::switch_context`.
pub unsafe fn switch(old: *mut CpuContext32, new: *const CpuContext32) {
    CpuContext32::switch(old, new);
}
