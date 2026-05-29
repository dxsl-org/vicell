//! ARM Generic Timer driver (EL1 Physical Timer).
//!
//! Uses CNTP_TVAL_EL0 / CNTP_CTL_EL0 available in EL1 on QEMU virt.

/// Ticks-per-quantum: ~10 ms at 62.5 MHz reference clock on QEMU virt.
const TICKS_PER_QUANTUM: u64 = 625_000;

/// Initialise and arm the physical timer.
pub fn init() {
    // SAFETY: timer system-register writes are EL1-private and do not affect memory safety.
    unsafe {
        // SAFETY: timer system-register writes are EL1-private; no memory invariants affected.
        // `msr` to a system register requires a GP register source — immediates not accepted.
        core::arch::asm!(
            "msr cntp_tval_el0, {val}",
            "mov {ctl}, #1",            // ENABLE bit
            "msr cntp_ctl_el0,  {ctl}",
            val = in(reg) TICKS_PER_QUANTUM,
            ctl = out(reg) _,
            options(nomem, nostack),
        );
    }
    // Enable the physical timer IRQ in the GIC (SPI 30 on QEMU virt).
    super::gic::enable_irq(30);
}

/// Re-arm the timer for the next quantum.  Call from the IRQ handler.
pub fn reset() {
    // SAFETY: same as init().
    unsafe {
        core::arch::asm!(
            "msr cntp_tval_el0, {val}",
            val = in(reg) TICKS_PER_QUANTUM,
            options(nomem, nostack),
        );
    }
}

/// Read the current cycle counter (CNTPCT_EL0).
pub fn read_ticks() -> u64 {
    let val: u64;
    // SAFETY: CNTPCT_EL0 is read-only; no state modified.
    unsafe { core::arch::asm!("mrs {}, cntpct_el0", out(reg) val, options(nomem, nostack)); }
    val
}
