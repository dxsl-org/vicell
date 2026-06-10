//! Local APIC (0xFEE0_0000) and I/O APIC (0xFEC0_0000) MMIO drivers.
//!
//! Limine does NOT identity-map MMIO regions; it only maps physical RAM via the
//! HHDM.  All LAPIC/IOAPIC accesses therefore use `HHDM_BASE + PHYS_ADDR`.
//! `HHDM_BASE` is stored at LAPIC init time from the Limine HHDM response.
const LAPIC_PHYS:  usize = 0xFEE0_0000;
const IOAPIC_PHYS: usize = 0xFEC0_0000;

use core::sync::atomic::{AtomicU64, Ordering};
static HHDM_BASE: AtomicU64 = AtomicU64::new(0);

/// Store the HHDM base address so all LAPIC/IOAPIC helpers can use it.
/// Must be called before any other function in this module.
pub fn set_hhdm_base(offset: u64) {
    HHDM_BASE.store(offset, Ordering::Relaxed);
}

#[inline]
fn lapic_base() -> usize {
    (HHDM_BASE.load(Ordering::Relaxed) as usize) + LAPIC_PHYS
}
#[inline]
fn ioapic_base() -> usize {
    (HHDM_BASE.load(Ordering::Relaxed) as usize) + IOAPIC_PHYS
}

fn lw(reg: usize, v: u32) {
    // SAFETY: LAPIC is accessible via HHDM_BASE + LAPIC_PHYS; write does not
    // affect memory safety.
    unsafe { core::ptr::write_volatile((lapic_base() + reg) as *mut u32, v); }
}
fn iow(idx: u8, v: u32) {
    // SAFETY: IOAPIC is accessible via HHDM_BASE + IOAPIC_PHYS.
    let base = ioapic_base();
    unsafe {
        core::ptr::write_volatile(base as *mut u32, idx as u32);
        core::ptr::write_volatile((base + 0x10) as *mut u32, v);
    }
}

/// Initialise LAPIC and configure periodic timer at ~100 Hz (vector 0x20).
pub fn init_lapic() {
    lw(0x0F0, 0x1FF);          // SVR: enable LAPIC, spurious vector 0xFF
    lw(0x3E0, 0x3);            // Timer divide-by-16
    lw(0x320, 0x20 | (1<<17)); // LVT_TIMER: periodic mode, vector 0x20
    lw(0x380, 1_000_000 / 16); // Initial count (~100 Hz at 1 GHz LAPIC clock)
}

/// Signal End-of-Interrupt to the LAPIC.
pub fn eoi() {
    lw(0x0B0, 0);
}

/// Start LAPIC one-shot timer at `count` for calibration (called by HPET driver).
///
/// # Safety
/// LAPIC must be identity-mapped and enabled.
pub unsafe fn start_oneshot(count: u32) {
    lw(0x3E0, 0x3);            // Timer divide-by-16
    lw(0x320, 0x20 | (1<<18)); // LVT_TIMER: one-shot mode, vector 0x20, masked
    lw(0x380, count);           // Initial count
}

/// Read the current LAPIC timer count (used by HPET calibration).
pub fn read_current_count() -> u32 {
    // SAFETY: LAPIC is accessible via HHDM_BASE + LAPIC_PHYS.
    unsafe { core::ptr::read_volatile((lapic_base() + 0x390) as *const u32) }
}

/// Re-initialise LAPIC timer with a calibrated ticks-per-ms value (~100 Hz).
///
/// Replaces the hardcoded initial count from `init_lapic`.
pub fn init_lapic_calibrated(ticks_per_ms: u64) {
    let count = (ticks_per_ms * 10) as u32; // 10 ms period for ~100 Hz
    lw(0x3E0, 0x3);             // Timer divide-by-16
    lw(0x320, 0x20 | (1<<17));  // LVT_TIMER: periodic, vector 0x20
    lw(0x380, count);
}

/// Redirect IOAPIC IRQ to IDT vector on CPU 0 (edge-triggered, active-high).
pub fn ioapic_redirect(irq: u8, vec: u8) {
    iow(0x10 + irq * 2 + 1, 0);       // destination: CPU 0
    iow(0x10 + irq * 2,     vec as u32); // vector, unmasked
}
