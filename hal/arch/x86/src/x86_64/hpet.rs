//! HPET (High Precision Event Timer) driver + PIT calibration fallback.
//!
//! Precondition: call `init()` with a valid virtual address for the HPET
//! MMIO registers (0xFED0_0000 via identity map after init_kernel_paging_x86).
//!
//! GCAP_ID[63:32] = CLK_PERIOD in femtoseconds (fs).
//! now_ns() = counter * CLK_PERIOD / 1_000_000  (fs → ns).
//!
//! If HPET hardware is absent (GCAP_ID reads 0), `calibrate_lapic` falls back
//! to an 8254 PIT channel-2 one-shot window (~10 ms).

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

// PIT port I/O helpers (needed for PIT-based calibration fallback).
#[inline] unsafe fn outb(port: u16, val: u8) {
    // SAFETY: port I/O is always valid at Ring 0.
    unsafe { core::arch::asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack)); }
}
#[inline] unsafe fn inb(port: u16) -> u8 {
    let v: u8;
    // SAFETY: port I/O is always valid at Ring 0.
    unsafe { core::arch::asm!("in al, dx", out("al") v, in("dx") port, options(nomem, nostack)); }
    v
}

/// Base virtual address of the HPET MMIO region (set by `init`).
static HPET_BASE: AtomicUsize = AtomicUsize::new(0);
/// Clock period in femtoseconds read from GCAP_ID[63:32].
static HPET_PERIOD_FS: AtomicU64 = AtomicU64::new(0);

/// HPET register offsets.
const GCAP_ID:       usize = 0x00; // General Capabilities and ID
const GEN_CONF:      usize = 0x10; // General Configuration
const MAIN_COUNTER:  usize = 0xF0; // Main Counter Value

fn read64(off: usize) -> u64 {
    let base = HPET_BASE.load(Ordering::Relaxed);
    // SAFETY: caller guarantees HPET_BASE is a valid MMIO VA.
    unsafe { core::ptr::read_volatile((base + off) as *const u64) }
}
fn write64(off: usize, val: u64) {
    let base = HPET_BASE.load(Ordering::Relaxed);
    // SAFETY: caller guarantees HPET_BASE is a valid MMIO VA.
    unsafe { core::ptr::write_volatile((base + off) as *mut u64, val) }
}

/// Initialise the HPET. `virt_base` must be the MMIO virtual address.
///
/// # Safety
/// `virt_base` must map to the HPET MMIO region and remain valid.
pub unsafe fn init(virt_base: usize) {
    HPET_BASE.store(virt_base, Ordering::Relaxed);
    let gcap = read64(GCAP_ID);
    let period_fs = gcap >> 32; // femtoseconds per tick
    HPET_PERIOD_FS.store(period_fs, Ordering::Relaxed);
    // Enable the HPET main counter (bit 0 of GEN_CONF).
    let conf = read64(GEN_CONF);
    write64(GEN_CONF, conf | 1);
}

/// Returns the current time in nanoseconds since HPET was enabled.
pub fn now_ns() -> u64 {
    let count = read64(MAIN_COUNTER);
    let period_fs = HPET_PERIOD_FS.load(Ordering::Relaxed);
    if period_fs == 0 { return 0; }
    // count * period_fs / 1_000_000 converts fs ticks → ns.
    // Use u128 to avoid overflow on long uptimes.
    ((count as u128 * period_fs as u128) / 1_000_000) as u64
}

/// Spin-wait for `ns` nanoseconds using the HPET main counter.
///
/// Returns immediately if HPET was not initialised (`period_fs == 0`).
/// Callers that need a real delay when HPET is absent must use PIT.
pub fn spin_ns(ns: u64) {
    if HPET_PERIOD_FS.load(Ordering::Relaxed) == 0 { return; }
    let start = now_ns();
    while now_ns().wrapping_sub(start) < ns {
        core::hint::spin_loop();
    }
}

/// PIT channel-2 one-shot calibration fallback (~10 ms reference window).
///
/// Uses 8254 PIT channel 2 in mode 0. The PIT clock is 1.193182 MHz;
/// 11932 counts ≈ 10 ms. Polls bit 5 of port 0x61 (ch2 output).
///
/// # Safety
/// Must be called from Ring 0. 8254 PIT is always present on x86/x86_64.
unsafe fn calibrate_lapic_pit() -> u64 {
    use super::apic;
    const PIT_CMD:    u16 = 0x43;
    const PIT_CH2:    u16 = 0x42;
    const SPKR_CTRL:  u16 = 0x61;
    const PIT_TICKS:  u16 = 11932; // 10 ms @ 1.193182 MHz
    const CAL_COUNT:  u32 = 0xFFFF_FFFF;

    unsafe {
        let orig = inb(SPKR_CTRL);
        // Disable gate (bit 0) to reset output before loading count.
        outb(SPKR_CTRL, orig & !0x01);
        // Channel 2, mode 0, lobyte/hibyte, binary: 0xB0.
        outb(PIT_CMD, 0xB0);
        outb(PIT_CH2, (PIT_TICKS & 0xFF) as u8);
        outb(PIT_CH2, (PIT_TICKS >> 8) as u8);
        // Arm LAPIC one-shot.
        apic::start_oneshot(CAL_COUNT);
        // Enable gate (set bit 0, keep speaker off).
        outb(SPKR_CTRL, (orig & !0x02) | 0x01);
        // Poll bit 5 (ch2 output) until HIGH (count reached zero).
        while inb(SPKR_CTRL) & 0x20 == 0 {
            core::hint::spin_loop();
        }
        // Restore gate.
        outb(SPKR_CTRL, orig & !0x01);
        let remaining = apic::read_current_count();
        let elapsed = CAL_COUNT - remaining;
        (elapsed as u64 + 5) / 10 // ticks per ms
    }
}

/// Calibrate the LAPIC timer using a 10 ms timing window.
///
/// Uses HPET if available; falls back to PIT channel 2 otherwise.
/// Returns LAPIC ticks per millisecond.
pub fn calibrate_lapic() -> u64 {
    if HPET_PERIOD_FS.load(Ordering::Relaxed) == 0 {
        // HPET absent or returned period_fs == 0 — use PIT fallback.
        // SAFETY: PIT is always present on x86/x86_64; Ring-0 port I/O is safe.
        unsafe { calibrate_lapic_pit() }
    } else {
        use super::apic;
        const CALIBRATION_COUNT: u32 = 0xFFFF_FFFF;
        // SAFETY: LAPIC is identity-mapped at 0xFEE0_0000 after init_kernel_paging_x86.
        unsafe { apic::start_oneshot(CALIBRATION_COUNT); }
        spin_ns(10_000_000); // 10 ms HPET window
        let remaining = apic::read_current_count();
        let elapsed_ticks = CALIBRATION_COUNT - remaining;
        (elapsed_ticks as u64 + 5) / 10 // ticks per ms (rounded)
    }
}
