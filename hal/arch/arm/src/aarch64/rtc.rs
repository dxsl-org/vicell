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
pub fn now_epoch_ns() -> u64 {
    let base = BASE.load(Ordering::Acquire);
    if base == 0 {
        return 0;
    }
    // SAFETY: base is a valid MMIO window; volatile reads are non-aliasing.
    unsafe {
        let low  = core::ptr::read_volatile(base as *const u32);
        let high = core::ptr::read_volatile((base + 4) as *const u32);
        (high as u64) << 32 | low as u64
    }
}
