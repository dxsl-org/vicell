//! x86_64 HAL facade.
//!
//! Sub-modules containing x86_64 assembly are gated on
//! `#[cfg(target_arch = "x86_64")]` to keep other targets' builds clean.

use hal_arch_trait::Arch;

#[cfg(target_arch = "x86_64")] pub mod boot;
#[cfg(target_arch = "x86_64")] pub mod context;
#[cfg(target_arch = "x86_64")] pub mod gdt;
#[cfg(target_arch = "x86_64")] pub mod hpet;
#[cfg(target_arch = "x86_64")] pub mod idt;
#[cfg(target_arch = "x86_64")] pub mod apic;
#[cfg(target_arch = "x86_64")] pub mod paging;
#[cfg(target_arch = "x86_64")] pub mod rtc;
#[cfg(target_arch = "x86_64")] pub mod syscall;
#[cfg(target_arch = "x86_64")] pub mod timer;
#[cfg(target_arch = "x86_64")] pub mod uart_16550;
#[cfg(target_arch = "x86_64")] pub mod trap;

#[cfg(target_arch = "x86_64")] pub use context::CpuContext as Context;
#[cfg(target_arch = "x86_64")] pub use paging::PageTable;
#[cfg(target_arch = "x86_64")] pub use paging::PAGE_SIZE;

/// Mirrors the rv64 `pub mod arch { ... }` consumed by kernel/src/task*.rs.
///
/// `init()` is intentionally a no-op: GDT/IDT/syscall/APIC are initialised
/// once via `ARCH.init()` in kmain. `task_entry_point` calls this per-task
/// to match the RISC-V per-hart stvec setup convention; on x86_64 the IDT is
/// global so no per-task re-init is needed.
#[cfg(target_arch = "x86_64")]
pub mod arch {
    pub use super::context::CpuContext as Context;
    pub use super::trap::{ViTrapFrame, get_gp_tp};
    pub use super::set_kernel_stack;

    /// Per-task arch init — no-op on x86_64 (global IDT set up in kmain).
    pub fn init() {}

    pub fn enable_interrupts() {
        // SAFETY: sti is a Ring-0 instruction; no memory invariants affected.
        unsafe { core::arch::asm!("sti", options(nomem, nostack)); }
    }

    extern "C" {
        /// Assembly trampoline that bootstraps new x86_64 threads.
        /// Defined in hal/arch/x86/src/x86_64/boot.rs (Phase 02).
        pub fn thread_trampoline();
    }
}

/// Post-paging timer init: HPET enable + calibrated LAPIC periodic timer.
///
/// Must be called AFTER init_kernel_paging so 0xFED0_0000 (HPET) and
/// 0xFEE0_0000 (LAPIC) are identity-mapped in the working PML4.
#[cfg(target_arch = "x86_64")]
pub fn init_timers() {
    // SAFETY: 0xFED0_0000 is identity-mapped by init_kernel_paging (MMIO block).
    unsafe { hpet::init(0xFED0_0000); }
    let ticks_per_ms = hpet::calibrate_lapic();
    apic::init_lapic_calibrated(ticks_per_ms);
}

/// x86_64 architecture implementation.
pub struct X86_64Arch;

pub type PlatformArch = X86_64Arch;
pub static ARCH: PlatformArch = X86_64Arch;

// ── Stub impl for non-x86_64 targets ──────────────────────────────────────────
#[cfg(not(target_arch = "x86_64"))]
impl Arch for X86_64Arch {
    type Context = usize;
    fn init(&self) {}
    unsafe fn switch_context(&self, _: *mut Self::Context, _: *const Self::Context) {}
    fn enable_interrupts(&self) {}
    fn disable_interrupts(&self) {}
    fn wait_for_interrupt(&self) {}
    fn interrupts_enabled(&self) -> bool { false }
}

// ── Full implementation for x86_64 ────────────────────────────────────────────
#[cfg(target_arch = "x86_64")]
impl Arch for X86_64Arch {
    type Context = context::CpuContext;

    /// Initialise x86_64 hardware: GDT, IDT, APIC, syscall MSRs.
    fn init(&self) {
        gdt::init();
        idt::init();
        syscall::init();
        apic::init_lapic();
    }

    /// # Safety
    /// Both pointers must point to valid, aligned `CpuContext` structs.
    unsafe fn switch_context(&self, old: *mut Self::Context, new: *const Self::Context) {
        // SAFETY: invariant upheld by caller.
        unsafe { context::switch(old, new); }
    }

    fn enable_interrupts(&self) {
        // SAFETY: sti is a standard Ring-0 instruction; no memory invariants affected.
        unsafe { core::arch::asm!("sti", options(nomem, nostack)); }
    }

    fn disable_interrupts(&self) {
        // SAFETY: cli is a standard Ring-0 instruction.
        unsafe { core::arch::asm!("cli", options(nomem, nostack)); }
    }

    fn wait_for_interrupt(&self) {
        // SAFETY: hlt halts the CPU until the next interrupt; no memory side effects.
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
    }

    fn interrupts_enabled(&self) -> bool {
        let rflags: u64;
        // SAFETY: pushfq/popfq read RFLAGS without modifying any visible state.
        unsafe {
            core::arch::asm!(
                "pushfq",
                "pop {f}",
                f = out(reg) rflags,
                options(nomem),
            );
        }
        rflags & (1 << 9) != 0 // IF bit
    }
}

/// Set the kernel-stack pointer in the TSS (for Ring-3 to Ring-0 transitions).
#[cfg(target_arch = "x86_64")]
pub fn set_kernel_stack(sp: usize) {
    // Update both TSS.rsp0 (for hardware interrupt stack switch) and
    // the per-CPU GS area (for syscall_entry swapgs-based stack switch).
    gdt::set_kernel_stack(sp as u64);
    syscall::set_kernel_stack(sp as u64);
}
