//! Goldfish RTC driver for AArch64 (QEMU virt machine).
//!
//! QEMU ARM64 virt: Goldfish RTC at 0x0901_0000 (discoverable via DTB
//! compatible = "google,goldfish-rtc"). Call `init_default()` to use the
//! QEMU virt default when DTB parsing is not yet available for ARM64.

use core::sync::atomic::{AtomicUsize, Ordering};

/// QEMU ARM64 virt machine Goldfish RTC MMIO base.
const QEMU_VIRT_RTC_BASE: usize = 0x0901_0000;

static BASE: AtomicUsize = AtomicUsize::new(0);

/// Initialize with a specific MMIO base address (from DTB).
///
/// # Precondition
/// `base` must point to a valid 4 KB MMIO window.
pub fn init(base: usize) {
    BASE.store(base, Ordering::Release);
}

/// Initialize with the QEMU virt default if no address was set yet.
pub fn init_default() {
    if BASE.load(Ordering::Acquire) == 0 {
        BASE.store(QEMU_VIRT_RTC_BASE, Ordering::Release);
    }
}

/// Nanoseconds since Unix epoch; `0` if RTC not initialized.
///
/// QEMU ARM virt uses the PL031 (ARM PrimeCell RTC) at this address.
/// PL031 RTCDR (offset 0x0) returns a 32-bit seconds count since the Unix
/// epoch (sourced from QEMU_CLOCK_REALTIME on the host).  Multiply by
/// 1_000_000_000 to convert to nanoseconds for the common hal::rtc contract.
pub fn now_epoch_ns() -> u64 {
    let base = BASE.load(Ordering::Acquire);
    if base == 0 {
        return 0;
    }
    // SAFETY: base is a valid MMIO window; volatile read is non-aliasing.
    let secs = unsafe { core::ptr::read_volatile(base as *const u32) };
    secs as u64 * 1_000_000_000
}
