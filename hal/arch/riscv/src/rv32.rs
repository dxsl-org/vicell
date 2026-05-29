//! RISC-V 32-bit (RV32) Hardware Abstraction Layer.
//!
//! Targets `riscv32imac-unknown-none-elf` (QEMU virt32, or bare-metal).
//! Sub-modules are gated on `#[cfg(target_arch = "riscv32")]` so building
//! for any other target still compiles this file without inline asm errors.

use hal_arch_trait::Arch;

/// RV32 architecture implementation.
pub struct RiscV32Arch;

pub type PlatformArch = RiscV32Arch;
pub static ARCH: PlatformArch = RiscV32Arch;

// ── Stub for non-RV32 targets ─────────────────────────────────────────────────
#[cfg(not(target_arch = "riscv32"))]
impl Arch for RiscV32Arch {
    type Context = usize;
    fn init(&self) {}
    unsafe fn switch_context(&self, _: *mut Self::Context, _: *const Self::Context) {}
    fn enable_interrupts(&self) {}
    fn disable_interrupts(&self) {}
    fn wait_for_interrupt(&self) {}
    fn interrupts_enabled(&self) -> bool { false }
}

// ── Full implementation for RV32 ──────────────────────────────────────────────
#[cfg(target_arch = "riscv32")]
impl Arch for RiscV32Arch {
    type Context = Rv32Context;

    fn init(&self) {
        // Set stvec to the trap entry (shared with rv32/ sub-module once implemented).
        // TODO: uncomment when rv32/trap.rs is wired: trap::init();
    }

    unsafe fn switch_context(&self, _old: *mut Self::Context, _new: *const Self::Context) {
        // TODO: delegate to rv32/context.rs once implemented.
        unimplemented!("RV32 context switch not yet implemented");
    }

    fn enable_interrupts(&self) {
        // SAFETY: csrsi sstatus SIE — standard RV32S mode interrupt enable.
        unsafe { core::arch::asm!("csrsi sstatus, 0x2", options(nomem, nostack)); }
    }

    fn disable_interrupts(&self) {
        // SAFETY: csrci sstatus SIE — standard RV32S mode interrupt disable.
        unsafe { core::arch::asm!("csrci sstatus, 0x2", options(nomem, nostack)); }
    }

    fn wait_for_interrupt(&self) {
        // SAFETY: wfi is safe and available in RV32S mode.
        unsafe { core::arch::asm!("wfi", options(nomem, nostack)); }
    }

    fn interrupts_enabled(&self) -> bool {
        let sstatus: u32;
        // SAFETY: reading sstatus does not modify any state.
        unsafe { core::arch::asm!("csrr {}, sstatus", out(reg) sstatus, options(nomem, nostack)); }
        sstatus & (1 << 1) != 0 // SIE bit
    }
}

/// Minimal CPU context for RV32 cooperative context switching.
/// Mirrors RV64 CpuContext but with 32-bit registers.
#[cfg(target_arch = "riscv32")]
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Rv32Context {
    pub s0: u32, pub s1: u32, pub s2: u32, pub s3: u32,
    pub s4: u32, pub s5: u32, pub s6: u32, pub s7: u32,
    pub s8: u32, pub s9: u32, pub s10: u32, pub s11: u32,
    pub ra: u32, pub sp: u32,
}

// SV32 page-table constants (used by RV32 paging sub-module, TODO Phase 21+).
/// Page size for SV32: 4 KB.
pub const PAGE_SIZE: usize = 4096;
/// Superpage size for SV32: 4 MB (level-1 leaf entry).
pub const SUPERPAGE_SIZE: usize = 4 * 1024 * 1024;
