//! x86_32 (IA-32) Hardware Abstraction Layer.
//!
//! Targets `i686-unknown-none` (custom JSON, bare-metal, multiboot1 via QEMU `-kernel`).
//! Nano profile: GDT + IDT init, COM1 UART, cooperative context switch.
//! Sub-modules gated on `#[cfg(target_arch = "x86")]` so this file compiles
//! on all hosts without emitting 32-bit-only inline asm.

use hal_arch_trait::Arch;

pub struct X86_32Arch;
pub type PlatformArch = X86_32Arch;
pub static ARCH: PlatformArch = X86_32Arch;

// ── Sub-modules (active only when cross-compiling to x86_32) ─────────────────
#[cfg(target_arch = "x86")]
pub mod boot;
#[cfg(target_arch = "x86")]
pub mod context;
#[cfg(target_arch = "x86")]
pub mod gdt;
#[cfg(target_arch = "x86")]
pub mod idt;
#[cfg(target_arch = "x86")]
pub mod uart_16550;

// ── Arch trait stub for non-x86 hosts (compile-time only) ────────────────────
#[cfg(not(target_arch = "x86"))]
impl Arch for X86_32Arch {
    type Context = usize;
    fn init(&self) {}
    unsafe fn switch_context(&self, _: *mut Self::Context, _: *const Self::Context) {}
    fn enable_interrupts(&self) {}
    fn disable_interrupts(&self) {}
    fn wait_for_interrupt(&self) {}
    fn interrupts_enabled(&self) -> bool { false }
}

// ── Full Arch implementation for x86_32 ──────────────────────────────────────
#[cfg(target_arch = "x86")]
impl Arch for X86_32Arch {
    type Context = context::CpuContext32;

    fn init(&self) {
        gdt::init();
        idt::init();
    }

    unsafe fn switch_context(&self, old: *mut Self::Context, new: *const Self::Context) {
        // SAFETY: callers guarantee valid, non-aliasing aligned pointers.
        context::switch(old, new);
    }

    fn enable_interrupts(&self) {
        // SAFETY: STI enables IRQs in EFLAGS; only called after GDT/IDT are set up.
        unsafe { core::arch::asm!("sti", options(nomem, nostack)); }
    }

    fn disable_interrupts(&self) {
        // SAFETY: CLI disables IRQs; standard kernel-mode operation.
        unsafe { core::arch::asm!("cli", options(nomem, nostack)); }
    }

    fn wait_for_interrupt(&self) {
        // SAFETY: HLT waits for the next interrupt; IRQs must be enabled first.
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
    }

    fn interrupts_enabled(&self) -> bool {
        let flags: u32;
        // SAFETY: PUSHFD/POPFD read EFLAGS without modifying machine state.
        unsafe { core::arch::asm!("pushfd; pop {}", out(reg) flags, options(nomem)); }
        flags & (1 << 9) != 0 // IF bit
    }
}

// ── `hal::arch` public API expected by kernel/src/task.rs ────────────────────
#[cfg(target_arch = "x86")]
pub mod arch {
    /// Minimal x86_32 trap frame (nano profile — never populated at runtime).
    ///
    /// Fields `regs`, `sepc`, `sstatus` mirror the RV64 layout so scheduler
    /// code that accesses them compiles. On x86_32 they are zero-initialised
    /// and never written by hardware.
    #[repr(C)]
    #[derive(Debug, Clone, Copy, Default)]
    pub struct ViTrapFrame {
        /// General-purpose register file (eax..edi + padding to 32 slots).
        pub regs:    [usize; 32],
        /// Instruction pointer (equiv. of RV64 sepc).
        pub sepc:    usize,
        /// EFLAGS (equiv. of RV64 sstatus).
        pub sstatus: usize,
    }

    /// Context type alias used by `task.rs` BOOT_CONTEXT.
    pub use super::context::CpuContext32 as Context;

    /// GP and TP registers (RISC-V concept; x86_32 has neither → (0, 0)).
    pub fn get_gp_tp() -> (usize, usize) { (0, 0) }

    /// Set kernel stack pointer in scratch register (x86_32 uses TSS; stub for nano).
    pub fn set_kernel_stack(_kernel_stack_top: usize) {}

    /// Architecture init — loads GDT + IDT.
    pub fn init() {
        super::gdt::init();
        super::idt::init();
    }

    /// Enable hardware interrupts (STI).
    pub fn enable_interrupts() {
        // SAFETY: STI is valid only after GDT/IDT have been loaded (init() called).
        unsafe { core::arch::asm!("sti", options(nomem, nostack)); }
    }

    // Thread trampoline — symbol provided by boot.rs global_asm.
    extern "C" { pub fn thread_trampoline(); }
}
