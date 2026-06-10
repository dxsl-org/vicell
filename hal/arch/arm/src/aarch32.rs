//! ARM AArch32 (ARMv7-A) Hardware Abstraction Layer.
//!
//! Targets `armv7a-none-eabi` bare-metal.
//! Sub-modules gated on `#[cfg(target_arch = "arm")]` to avoid AArch32 inline
//! asm bleeding into non-ARM builds (e.g. AArch64 host compilations).

use hal_arch_trait::Arch;

pub struct AArch32Arch;
pub type PlatformArch = AArch32Arch;
pub static ARCH: PlatformArch = AArch32Arch;

// ── Sub-modules (active only when cross-compiling to arm) ────────────────────
#[cfg(target_arch = "arm")]
pub mod boot;
#[cfg(target_arch = "arm")]
pub mod context;
pub mod uart_pl011;

// ── Arch trait stub for non-AArch32 hosts ────────────────────────────────────
#[cfg(not(target_arch = "arm"))]
impl Arch for AArch32Arch {
    type Context = usize;
    fn init(&self) {}
    unsafe fn switch_context(&self, _: *mut Self::Context, _: *const Self::Context) {}
    fn enable_interrupts(&self) {}
    fn disable_interrupts(&self) {}
    fn wait_for_interrupt(&self) {}
    fn interrupts_enabled(&self) -> bool { false }
}

// ── Full Arch implementation for AArch32 ─────────────────────────────────────
#[cfg(target_arch = "arm")]
impl Arch for AArch32Arch {
    type Context = context::Arm32Context;

    /// Nano bringup: ARM virt GIC init deferred; UART already initialised in kmain.
    fn init(&self) {}

    unsafe fn switch_context(&self, old: *mut Self::Context, new: *const Self::Context) {
        // SAFETY: callers guarantee valid, non-aliasing, aligned pointers.
        context::switch(old, new);
    }

    fn enable_interrupts(&self) {
        // SAFETY: cpsie i — enables IRQ in CPSR; standard ARMv7 SVC-mode instruction.
        unsafe { core::arch::asm!("cpsie i", options(nomem, nostack)); }
    }

    fn disable_interrupts(&self) {
        // SAFETY: cpsid i — disables IRQ in CPSR.
        unsafe { core::arch::asm!("cpsid i", options(nomem, nostack)); }
    }

    fn wait_for_interrupt(&self) {
        // SAFETY: wfi is a standard ARMv7 idle instruction.
        unsafe { core::arch::asm!("wfi", options(nomem, nostack)); }
    }

    fn interrupts_enabled(&self) -> bool {
        let cpsr: u32;
        // SAFETY: mrs reads CPSR without modifying any state.
        unsafe { core::arch::asm!("mrs {}, cpsr", out(reg) cpsr, options(nomem, nostack)); }
        cpsr & (1 << 7) == 0 // I bit clear = IRQ enabled
    }
}

// ── AArch32 context type (used by Arch trait and `arch` module) ───────────────
/// Callee-saved register frame for ARMv7-A cooperative task switch.
///
/// Shared definition; must match the offsets in `context::__switch_arm32` asm.
#[cfg(target_arch = "arm")]
pub use context::Arm32Context;

/// Page size constant (same as AArch64 and RISC-V — 4 KiB pages).
pub const PAGE_SIZE: usize = 4096;
pub const SECTION_SIZE: usize = 1024 * 1024;

// ── `hal::arch` public API expected by kernel/src/task.rs ────────────────────
#[cfg(target_arch = "arm")]
pub mod arch {
    /// Minimal ARMv7-A trap frame (nano profile — never populated at runtime).
    ///
    /// Fields `regs`, `sepc`, `sstatus` mirror the RV64 layout so scheduler
    /// code that accesses them compiles. On AArch32 they are zero-initialised
    /// and never written by hardware.
    #[repr(C)]
    #[derive(Debug, Clone, Copy, Default)]
    pub struct ViTrapFrame {
        /// General-purpose register file (r0-r15 + padding to 32 slots).
        pub regs:    [usize; 32],
        /// Program counter (equiv. of RV64 sepc / ELR_EL1).
        pub sepc:    usize,
        /// Status register (equiv. of RV64 sstatus / SPSR).
        pub sstatus: usize,
    }

    /// Context type alias used by `task.rs` BOOT_CONTEXT.
    pub use super::context::Arm32Context as Context;

    /// GP and TP registers (RISC-V concept; ARM has neither → (0, 0)).
    pub fn get_gp_tp() -> (usize, usize) { (0, 0) }

    /// Set kernel stack (ARM uses banked registers; stub for nano profile).
    pub fn set_kernel_stack(_kernel_stack_top: usize) {}

    /// Architecture init — nano profile: GIC/timer deferred; no-op here.
    pub fn init() {}

    /// Enable IRQs in CPSR (cpsie i).
    pub fn enable_interrupts() {
        // SAFETY: standard ARMv7 SVC-mode instruction; only called after boot init.
        unsafe { core::arch::asm!("cpsie i", options(nomem, nostack)); }
    }

    // Thread trampoline — symbol provided by boot.rs global_asm.
    extern "C" { pub fn thread_trampoline(); }
}
