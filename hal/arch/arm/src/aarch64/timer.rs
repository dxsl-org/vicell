//! ARM Generic Timer driver.
//!
//! EL1 (default):  CNTP_TVAL_EL0 / CNTP_CTL_EL0 / PPI 30.
//! EL2 host:       CNTHP_TVAL_EL2 / CNTHP_CTL_EL2 / PPI 26.
//!
//! Runtime dispatch via `el2::is_el2()` (set before `kmain` in boot.rs).

/// Ticks-per-quantum: ~10 ms at 62.5 MHz reference clock on QEMU virt.
const TICKS_PER_QUANTUM: u64 = 625_000;

/// Initialise and arm the physical timer.
///
/// Selects the EL2 hypervisor physical timer when `el2::is_el2()` is true,
/// otherwise falls back to the EL1 physical timer.
pub fn init() {
    if super::el2::is_el2() {
        // SAFETY: CNTHP_* are EL2-private hypervisor physical timer registers;
        // no memory invariants affected.
        unsafe {
            core::arch::asm!(
                "msr cnthp_tval_el2, {val}",
                "mov {ctl}, #1",            // ENABLE bit
                "msr cnthp_ctl_el2,  {ctl}",
                val = in(reg) TICKS_PER_QUANTUM,
                ctl = out(reg) _,
                options(nomem, nostack),
            );
        }
        // Enable the hypervisor physical timer PPI in the GIC (PPI 26 on QEMU virt).
        super::gic::enable_irq(26);
    } else {
        // SAFETY: timer system-register writes are EL1-private; no memory invariants affected.
        unsafe {
            core::arch::asm!(
                "msr cntp_tval_el0, {val}",
                "mov {ctl}, #1",            // ENABLE bit
                "msr cntp_ctl_el0,  {ctl}",
                val = in(reg) TICKS_PER_QUANTUM,
                ctl = out(reg) _,
                options(nomem, nostack),
            );
        }
        // Enable the physical timer IRQ in the GIC (PPI 30 on QEMU virt).
        super::gic::enable_irq(30);
    }
}

/// Re-arm the timer for the next quantum.  Call from the IRQ handler.
///
/// Mirrors `init()` dispatch: CNTHP_TVAL_EL2 at EL2, CNTP_TVAL_EL0 at EL1.
pub fn reset() {
    if super::el2::is_el2() {
        // SAFETY: same as init() EL2 branch.
        unsafe {
            core::arch::asm!(
                "msr cnthp_tval_el2, {val}",
                val = in(reg) TICKS_PER_QUANTUM,
                options(nomem, nostack),
            );
        }
    } else {
        // SAFETY: same as init() EL1 branch.
        unsafe {
            core::arch::asm!(
                "msr cntp_tval_el0, {val}",
                val = in(reg) TICKS_PER_QUANTUM,
                options(nomem, nostack),
            );
        }
    }
}

/// Read the current cycle counter (CNTPCT_EL0).
pub fn read_ticks() -> u64 {
    let val: u64;
    // SAFETY: CNTPCT_EL0 is read-only; no state modified.
    unsafe { core::arch::asm!("mrs {}, cntpct_el0", out(reg) val, options(nomem, nostack)); }
    val
}
