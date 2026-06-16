//! Goldfish RTC driver for RISC-V (QEMU virt machine).
//!
//! Init: `platform::init()` calls `init(base)` with the MMIO base from DTB
//! node `google,goldfish-rtc`. MMIO layout (QEMU hw/rtc/goldfish_rtc.c):
//!   offset 0x00: TIME_LOW  — reading latches TIME_HIGH; returns bits 31:0 of epoch_ns
//!   offset 0x04: TIME_HIGH — returns latched bits 63:32 of epoch_ns

use core::sync::atomic::{AtomicUsize, Ordering};

static BASE: AtomicUsize = AtomicUsize::new(0);

/// Store the Goldfish RTC MMIO base address from the DTB.
///
/// # Precondition
/// `base` must point to a valid 4 KB MMIO window mapped in the kernel address space.
pub fn init(base: usize) {
    BASE.store(base, Ordering::Release);
}

/// Nanoseconds since Unix epoch (1970-01-01 00:00:00 UTC).
///
/// Returns `0` if `init()` was not called (no RTC found in DTB).
pub fn now_epoch_ns() -> u64 {
    let base = BASE.load(Ordering::Acquire);
    if base == 0 {
        return 0;
    }
    // Reading TIME_LOW (offset 0) first causes the hardware to latch TIME_HIGH.
    // SAFETY: base is a valid MMIO window set by init(); volatile reads are non-aliasing.
    unsafe {
        let low  = core::ptr::read_volatile(base as *const u32);
        let high = core::ptr::read_volatile((base + 4) as *const u32);
        (high as u64) << 32 | low as u64
    }
}
