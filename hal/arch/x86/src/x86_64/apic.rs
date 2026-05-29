//! Local APIC (0xFEE0_0000) and I/O APIC (0xFEC0_0000) MMIO drivers.
//!
//! Precondition: call `init_lapic()` while the Limine identity map is active
//! (before `PageTable::activate`).  After paging activation the kernel must
//! explicitly map these addresses if it accesses LAPIC/IOAPIC again.
const LAPIC:  usize = 0xFEE0_0000;
const IOAPIC: usize = 0xFEC0_0000;

fn lw(reg: usize, v: u32) {
    // SAFETY: LAPIC MMIO is identity-mapped; write does not affect memory safety.
    unsafe { core::ptr::write_volatile((LAPIC + reg) as *mut u32, v); }
}
fn iow(idx: u8, v: u32) {
    // SAFETY: IOAPIC MMIO is identity-mapped.
    unsafe {
        core::ptr::write_volatile(IOAPIC as *mut u32, idx as u32);
        core::ptr::write_volatile((IOAPIC + 0x10) as *mut u32, v);
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

/// Redirect IOAPIC IRQ to IDT vector on CPU 0 (edge-triggered, active-high).
pub fn ioapic_redirect(irq: u8, vec: u8) {
    iow(0x10 + irq * 2 + 1, 0);       // destination: CPU 0
    iow(0x10 + irq * 2,     vec as u32); // vector, unmasked
}
