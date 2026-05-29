//! ARM AArch32 (ARMv7-A) Hardware Abstraction Layer.
//!
//! Targets `armv7a-none-eabi` or `thumbv7em-none-eabihf` bare-metal.
//! Sub-modules gated on `#[cfg(target_arch = "arm")]` to avoid AArch32 inline
//! asm bleeding into non-ARM builds.

use hal_arch_trait::Arch;

/// AArch32 (ARMv7-A) architecture implementation.
pub struct AArch32Arch;

pub type PlatformArch = AArch32Arch;
pub static ARCH: PlatformArch = AArch32Arch;

// ── Stub for non-AArch32 targets ──────────────────────────────────────────────
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

// ── Full implementation for AArch32 ──────────────────────────────────────────
#[cfg(target_arch = "arm")]
impl Arch for AArch32Arch {
    type Context = Arm32Context;

    fn init(&self) {
        // TODO: set VBAR (vector base), enable GIC, init UART PL011.
    }

    unsafe fn switch_context(&self, _old: *mut Self::Context, _new: *const Self::Context) {
        // TODO: delegate to aarch32/context.rs once implemented.
        unimplemented!("AArch32 context switch not yet implemented");
    }

    fn enable_interrupts(&self) {
        // SAFETY: cpsie i — enables IRQ in CPSR; standard ARMv7 instruction.
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

/// Minimal CPU context for AArch32 cooperative context switching.
/// Saves AAPCS callee-saved registers (R4-R11) + SP + LR.
#[cfg(target_arch = "arm")]
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Arm32Context {
    pub r4: u32, pub r5: u32, pub r6: u32, pub r7: u32,
    pub r8: u32, pub r9: u32, pub r10: u32, pub r11: u32,
    pub sp: u32, // SP (R13)
    pub lr: u32, // LR (R14), return address
}

/// Short-descriptor paging constants for ARMv7-A (Sv7 / VMSA).
/// Section size: 1 MB.  Small page: 4 KB.
pub const PAGE_SIZE: usize = 4096;
pub const SECTION_SIZE: usize = 1024 * 1024;
