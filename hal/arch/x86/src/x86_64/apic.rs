//! Local APIC and I/O APIC MMIO drivers.
//!
//! Limine does NOT identity-map MMIO regions; it only maps physical RAM via the
//! HHDM.  All LAPIC/IOAPIC accesses use `HHDM_BASE + PHYS_ADDR`.
//! `HHDM_BASE` is reset to 0 after `init_kernel_paging_x86` identity-maps MMIO.
//! Physical bases default to QEMU q35 values; override from ACPI MADT via
//! `set_lapic_phys` / `set_ioapic_phys` before `init_timers()`.
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

static HHDM_BASE:   AtomicU64   = AtomicU64::new(0);
static LAPIC_PHYS:  AtomicUsize = AtomicUsize::new(0xFEE0_0000);
static IOAPIC_PHYS: AtomicUsize = AtomicUsize::new(0xFEC0_0000);

/// ISA IRQ → GSI override table from ACPI MADT type-2 entries.
/// Index = ISA IRQ (0–15); value = GSI. Identity-mapped by default.
static IRQ_OVERRIDES: [AtomicU32; 16] = [
    AtomicU32::new(0),  AtomicU32::new(1),  AtomicU32::new(2),  AtomicU32::new(3),
    AtomicU32::new(4),  AtomicU32::new(5),  AtomicU32::new(6),  AtomicU32::new(7),
    AtomicU32::new(8),  AtomicU32::new(9),  AtomicU32::new(10), AtomicU32::new(11),
    AtomicU32::new(12), AtomicU32::new(13), AtomicU32::new(14), AtomicU32::new(15),
];
/// GSI base of the first I/O APIC (MADT type-1 gsi_base field). Usually 0.
static IOAPIC_GSI_BASE: AtomicU32 = AtomicU32::new(0);

/// Store the HHDM base address so all LAPIC/IOAPIC helpers can use it.
/// Must be called before any other function in this module.
pub fn set_hhdm_base(offset: u64) {
    HHDM_BASE.store(offset, Ordering::Relaxed);
}

/// Override the LAPIC physical base from ACPI MADT. Defaults to 0xFEE0_0000.
/// Call before `init_timers()`.
pub fn set_lapic_phys(base: u64) {
    LAPIC_PHYS.store(base as usize, Ordering::Relaxed);
}

/// Override the I/O APIC physical base from ACPI MADT. Defaults to 0xFEC0_0000.
/// Call before `init_timers()`.
pub fn set_ioapic_phys(base: u64) {
    IOAPIC_PHYS.store(base as usize, Ordering::Relaxed);
}

/// Store MADT type-2 IRQ source overrides and the IOAPIC GSI base.
///
/// `overrides[n]` = GSI for ISA IRQ n; `gsi_base` = first GSI owned by this
/// IOAPIC (MADT type-1 field). Call before `init_input_irq()`.
pub fn set_irq_overrides(overrides: &[u32; 16], gsi_base: u32) {
    IOAPIC_GSI_BASE.store(gsi_base, Ordering::Relaxed);
    for (i, &gsi) in overrides.iter().enumerate() {
        IRQ_OVERRIDES[i].store(gsi, Ordering::Relaxed);
    }
}

#[inline]
fn lapic_base() -> usize {
    (HHDM_BASE.load(Ordering::Relaxed) as usize) + LAPIC_PHYS.load(Ordering::Relaxed)
}
#[inline]
fn ioapic_base() -> usize {
    (HHDM_BASE.load(Ordering::Relaxed) as usize) + IOAPIC_PHYS.load(Ordering::Relaxed)
}

fn lw(reg: usize, v: u32) {
    // SAFETY: LAPIC is identity-mapped by init_kernel_paging_x86 at the parsed base.
    unsafe { core::ptr::write_volatile((lapic_base() + reg) as *mut u32, v); }
}
fn iow(idx: u8, v: u32) {
    // SAFETY: IOAPIC is identity-mapped by init_kernel_paging_x86 at the parsed base.
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
    // bits 18:17 = 00 → one-shot mode; bit 16 = 1 → masked (silent calibration)
    lw(0x320, 0x20 | (1<<16)); // LVT_TIMER: one-shot, masked, vector 0x20
    lw(0x380, count);           // Initial count
}

/// Read the current LAPIC timer count (used by HPET calibration).
pub fn read_current_count() -> u32 {
    // SAFETY: LAPIC is identity-mapped at LAPIC_PHYS after init_kernel_paging_x86.
    unsafe { core::ptr::read_volatile((lapic_base() + 0x390) as *const u32) }
}

/// Read the LVT Timer register (for debug/diagnostic only).
pub fn read_lvt_timer() -> u32 {
    // SAFETY: LAPIC is identity-mapped at LAPIC_PHYS after init_kernel_paging_x86.
    unsafe { core::ptr::read_volatile((lapic_base() + 0x320) as *const u32) }
}

/// Read the LAPIC initial count register (0x380).
pub fn read_initial_count() -> u32 {
    // SAFETY: LAPIC is identity-mapped at LAPIC_PHYS after init_kernel_paging_x86.
    unsafe { core::ptr::read_volatile((lapic_base() + 0x380) as *const u32) }
}

/// Check if x2APIC mode is active (IA32_APIC_BASE MSR bit 10).
///
/// When x2APIC is enabled the MMIO interface at 0xFEE0_0000 is DISABLED.
/// All `lw()` writes are silently lost; the LAPIC is unreachable via MMIO.
pub fn check_x2apic() -> bool {
    let lo: u32;
    // SAFETY: rdmsr from Ring 0; IA32_APIC_BASE (0x1B) is always readable.
    unsafe {
        core::arch::asm!(
            "rdmsr",
            in("ecx") 0x1Bu32,
            out("eax") lo,
            out("edx") _,
        );
    }
    lo & (1 << 10) != 0  // bit 10 = x2APIC enable
}

/// Re-initialise LAPIC timer with a calibrated ticks-per-ms value (~100 Hz).
///
/// Replaces the hardcoded initial count from `init_lapic`.
/// Safety: if calibration returned 0 or the 10× product overflows u32 (ticks_per_ms
/// > 429_496_729), fall back to a hardcoded estimate.
/// Writing count=0 to the LAPIC initial-count register STOPS the timer entirely.
/// A tiny count (< 10_000) means ~160 μs period → recursive ISR storm → crash.
pub fn init_lapic_calibrated(ticks_per_ms: u64) {
    // u32::MAX / 10 = 429_496_729.  If ticks_per_ms exceeds this, the cast wraps
    // to a tiny value (e.g. 429_496_730 × 10 = 4_294_967_300 → cast = 4) which
    // fires the timer every ~64 ns and causes an immediate recursive-ISR storm.
    const FALLBACK: u32 = 62_500; // ~10 ms at 100 MHz LAPIC / divide-16
    let safe_count = if ticks_per_ms == 0 || ticks_per_ms > (u32::MAX as u64 / 10) {
        FALLBACK
    } else {
        let c = (ticks_per_ms * 10) as u32;
        if c < 10_000 { FALLBACK } else { c }
    };
    lw(0x3E0, 0x3);             // Timer divide-by-16
    lw(0x320, 0x20 | (1<<17));  // LVT_TIMER: periodic, vector 0x20
    lw(0x380, safe_count);
}

/// Redirect an ISA IRQ to an IDT vector on CPU 0 (edge-triggered, active-high).
///
/// Translates `isa_irq` through the MADT type-2 override table to the correct
/// GSI, then subtracts the IOAPIC's GSI base to get the physical pin index.
/// On QEMU q35 the override table is identity-mapped (no change in behaviour).
pub fn ioapic_redirect(isa_irq: u8, vec: u8) {
    let gsi = IRQ_OVERRIDES
        .get(isa_irq as usize)
        .map(|a| a.load(Ordering::Relaxed))
        .unwrap_or(isa_irq as u32);
    let gsi_base = IOAPIC_GSI_BASE.load(Ordering::Relaxed);
    let pin = gsi.saturating_sub(gsi_base) as u8;
    iow(0x10 + pin * 2 + 1, 0);        // destination: CPU 0
    iow(0x10 + pin * 2,     vec as u32); // vector, unmasked
}
