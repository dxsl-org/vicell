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

/// Post-paging timer init: LAPIC enable + HPET/PIT calibration.
///
/// Must be called AFTER init_kernel_paging_x86, which identity-maps:
///   0xFEC0_0000 (IOAPIC), 0xFED0_0000 (HPET), 0xFEE0_0000 (LAPIC).
///
/// Resets the APIC HHDM_BASE to 0 so lapic_base() == 0xFEE0_0000 (identity).
/// Limine does not include MMIO in its HHDM for small RAM machines, so the
/// HHDM-offset LAPIC address would fault; identity-map is always correct.
#[cfg(target_arch = "x86_64")]
pub fn init_timers() {
    // Switch LAPIC/IOAPIC accesses to the identity-mapped PAs we set up in our PML4.
    apic::set_hhdm_base(0);

    // Enable LAPIC (SVR) and set an initial periodic timer config.
    apic::init_lapic();
    uart_16550::putchar(b'A'); // LAPIC enabled

    // Init HPET. If hardware absent, HPET_PERIOD_FS stays 0 and calibrate_lapic
    // falls back to the 8254 PIT path automatically.
    // SAFETY: 0xFED0_0000 is identity-mapped by init_kernel_paging_x86.
    unsafe { hpet::init(0xFED0_0000); }
    uart_16550::putchar(b'H'); // after HPET init (H=HPET attempted)

    let ticks_per_ms = hpet::calibrate_lapic();
    uart_16550::putchar(b'C'); // calibration done
    // '0'=zero (timer disabled!), '1'=ok.
    uart_16550::putchar(if ticks_per_ms == 0 { b'0' } else { b'1' });
    // 'O'=overflow (>u32::MAX/10 → count wraps to tiny value → ISR storm), 'o'=ok.
    uart_16550::putchar(if ticks_per_ms > (u32::MAX as u64 / 10) { b'O' } else { b'o' });

    apic::init_lapic_calibrated(ticks_per_ms);

    // Read back LVT_TIMER and initial count to verify LAPIC is armed.
    // 'K'=unmasked+periodic, 'M'=masked, '?'=unexpected.
    let lvt = apic::read_lvt_timer();
    let init_cnt = apic::read_initial_count();
    let lvt_masked = (lvt >> 16) & 1 != 0;        // bit 16 = mask
    let lvt_mode   = (lvt >> 17) & 0x3;            // bits 18:17 = timer mode
    uart_16550::putchar(if lvt_masked { b'M' } else if lvt_mode == 1 { b'K' } else { b'?' });
    // Print initial count as a range probe:
    // '!'=0 (stopped), 'l'=<10000, 's'=<100000, 'n'=<1000000, 'b'=>=1000000
    uart_16550::putchar(match init_cnt {
        0             => b'!',
        1..=9_999     => b'l',
        10_000..=99_999 => b's',
        100_000..=999_999 => b'n',
        _ => b'b',
    });

    // Check IA32_APIC_BASE MSR for x2APIC mode.
    // 'x' = xAPIC (MMIO works) / 'X' = x2APIC active (MMIO is DISABLED — all lw() calls lost).
    uart_16550::putchar(if apic::check_x2apic() { b'X' } else { b'x' });

    // Verify the timer fires from ring-0 before entering user mode.
    // Enable interrupts + halt twice.  'V' probes from x86_64_timer_handler appear
    // before '~' if the LAPIC is delivering correctly from ring-0.
    // SAFETY: sti/hlt/cli are Ring-0 instructions; no memory side effects.
    for _ in 0..2u8 {
        unsafe {
            core::arch::asm!(
                "sti",
                "hlt",   // sleep until next interrupt (LAPIC timer or other)
                "cli",
                options(nomem, nostack)
            );
        }
        uart_16550::putchar(b'~');  // '~' = returned from this hlt iteration
    }
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
