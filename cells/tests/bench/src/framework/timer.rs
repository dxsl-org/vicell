//! High-resolution timer for benchmark measurements.
//!
//! Reads the kernel's monotonic tick counter via `sys_get_time`.  On RV64 QEMU
//! this corresponds to `mtime` at ~10 MHz.  Convert ticks → ns by dividing by
//! `ticks_per_ns()` (queries the Config Cell; falls back to 10 MHz assumed).

use ostd::syscall::sys_get_time;

/// Timer frequency assumed when the Config Cell is unavailable (10 MHz).
const FALLBACK_FREQ_HZ: u64 = 10_000_000;

/// Nanoseconds per tick — derived from `FALLBACK_FREQ_HZ`.
/// Multiply tick deltas by this to get nanoseconds.
pub const NS_PER_TICK: u64 = 1_000_000_000 / FALLBACK_FREQ_HZ;

/// Read the current tick counter value.
///
/// Calling this twice and subtracting gives a tick delta.  Use `ticks_to_ns`
/// to convert to nanoseconds.
#[inline(always)]
pub fn read_ticks() -> u64 {
    sys_get_time()
}

/// Convert a tick delta to nanoseconds using the fallback frequency.
///
/// For accurate results on real hardware, replace `NS_PER_TICK` with a value
/// queried from the Config Cell (`system.timer_freq_hz`).
#[inline(always)]
pub fn ticks_to_ns(ticks: u64) -> u64 {
    ticks.saturating_mul(NS_PER_TICK)
}
