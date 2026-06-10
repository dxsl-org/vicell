//! AArch64 HAL facade.
//!
//! All sub-modules containing AArch64 assembly are gated on
//! `#[cfg(target_arch = "aarch64")]` so the workspace compiles for other targets.

use hal_arch_trait::Arch;

// Sub-modules only compile when targeting AArch64.
#[cfg(target_arch = "aarch64")] pub mod boot;
#[cfg(target_arch = "aarch64")] pub mod context;
#[cfg(target_arch = "aarch64")] pub mod gic;
#[cfg(target_arch = "aarch64")] pub mod paging;
#[cfg(target_arch = "aarch64")] pub mod rtc;
#[cfg(target_arch = "aarch64")] pub mod timer;
#[cfg(target_arch = "aarch64")] pub mod trap;
#[cfg(target_arch = "aarch64")] pub mod uart_pl011;

#[cfg(target_arch = "aarch64")] pub use context::CpuContext as Context;
#[cfg(target_arch = "aarch64")] pub use paging::PageTable;
#[cfg(target_arch = "aarch64")] pub use paging::PAGE_SIZE;

// ── Stub for non-AArch64 targets ─────────────────────────────────────────────

/// AArch64 architecture stub.
pub struct AArch64Arch;

pub type PlatformArch = AArch64Arch;
pub static ARCH: PlatformArch = AArch64Arch;

#[cfg(not(target_arch = "aarch64"))]
impl Arch for AArch64Arch {
    type Context = usize;
    fn init(&self) {}
    unsafe fn switch_context(&self, _old: *mut Self::Context, _new: *const Self::Context) {}
    fn enable_interrupts(&self) {}
    fn disable_interrupts(&self) {}
    fn wait_for_interrupt(&self) {}
    fn interrupts_enabled(&self) -> bool { false }
}

// ── Full implementation for AArch64 target ────────────────────────────────────

#[cfg(target_arch = "aarch64")]
impl Arch for AArch64Arch {
    type Context = context::CpuContext;

    fn init(&self) {
        trap::init();
    }

    unsafe fn switch_context(&self, old: *mut Self::Context, new: *const Self::Context) {
        // SAFETY: invariant upheld by caller.
        unsafe { context::switch(old, new); }
    }

    fn enable_interrupts(&self) {
        // SAFETY: daifclr is a standard EL1 control write.
        unsafe { core::arch::asm!("msr daifclr, #2", options(nomem, nostack)); }
    }

    fn disable_interrupts(&self) {
        // SAFETY: daifset is a standard EL1 control write.
        unsafe { core::arch::asm!("msr daifset, #2", options(nomem, nostack)); }
    }

    fn wait_for_interrupt(&self) {
        // SAFETY: wfi has no side-effects on memory.
        unsafe { core::arch::asm!("wfi", options(nomem, nostack)); }
    }

    fn interrupts_enabled(&self) -> bool {
        let daif: u64;
        // SAFETY: reading DAIF modifies no state.
        unsafe { core::arch::asm!("mrs {}, daif", out(reg) daif, options(nomem, nostack)); }
        (daif & (1 << 7)) == 0
    }
}

/// Store the kernel-stack top in TPIDR_EL1 (AArch64 only).
#[cfg(target_arch = "aarch64")]
pub fn set_kernel_stack(sp: usize) {
    // SAFETY: TPIDR_EL1 is EL1-private; writing from EL1 is safe.
    unsafe { core::arch::asm!("msr tpidr_el1, {}", in(reg) sp, options(nomem, nostack)); }
}

/// `hal::arch` shim — exposes a RISC-V-compatible API surface so the ViCell
/// kernel compiles for aarch64 without #[cfg] sprawl in the common scheduler code.
/// Field-name differences (ra/s0/s1 vs x30/x19/x20) are still cfg-gated at
/// the call sites in task.rs and scheduler.rs.
#[cfg(target_arch = "aarch64")]
pub mod arch {
    /// Unified abstract trap frame — same field names as the RISC-V ViTrapFrame
    /// so the kernel's scheduling code compiles without modification.
    /// `sepc` maps to ELR_EL1, `sstatus` to SPSR_EL1.
    #[derive(Default, Clone, Copy, Debug)]
    #[repr(C)]
    pub struct ViTrapFrame {
        pub regs:    [usize; 32],
        pub sstatus: usize,
        pub sepc:    usize,
        pub stval:   usize,
        pub scause:  usize,
    }

    pub use super::context::CpuContext as Context;

    /// Initialise the ARM64 exception vector table.
    pub fn init() { super::trap::init(); }

    /// Enable IRQs by clearing DAIF.I.
    pub fn enable_interrupts() { super::trap::enable_interrupts(); }

    /// ARM64 stores the kernel-stack pointer in TPIDR_EL1.
    pub fn set_kernel_stack(sp: usize) { super::set_kernel_stack(sp); }

    /// ARM64 has no GP/TP registers; return zeroes for spawn compatibility.
    pub fn get_gp_tp() -> (usize, usize) { (0, 0) }

    extern "C" {
        /// Entry trampoline for new tasks (enables IRQs, sets x0=arg, jumps to entry).
        pub fn thread_trampoline();
    }
}
