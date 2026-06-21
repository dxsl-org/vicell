//! ViTlsClock — TlsClock impl backed by the hardware RTC syscall.
//!
//! Contract: `now()` returns `Some(epoch_seconds)` clamped to at least
//! `VICELL_MIN_UNIX` so that a missing or unsynced RTC does not cause every
//! certificate to appear expired.  It NEVER returns `None` — an absent RTC
//! produces the floor value, which is a known-safe past date, not a bypass.
//!
//! The floor is intentionally conservative: it only protects against epoch-0
//! (RTC unset).  Valid future dates always override the floor.

use embedded_tls::TlsClock;
use ostd::syscall::sys_get_wall_secs;

/// Fixed floor: 2025-06-01T00:00:00Z.
///
/// Injected by build.rs via `VICELL_MIN_UNIX` (may use SOURCE_DATE_EPOCH for
/// reproducible builds).  Fallback is the compile-time constant below.
const VICELL_MIN_UNIX: u64 = {
    // option_env! returns None at runtime; resolve at compile time.
    // The value is emitted by build.rs as a string env var.
    match option_env!("VICELL_MIN_UNIX") {
        Some(s) => {
            // const parse u64 from decimal string — no std needed.
            let bytes = s.as_bytes();
            let mut val: u64 = 0;
            let mut i = 0;
            while i < bytes.len() {
                val = val * 10 + (bytes[i] - b'0') as u64;
                i += 1;
            }
            val
        }
        None => 1_748_736_000,
    }
};

/// A TLS clock that reads Unix epoch seconds from the hardware RTC.
///
/// Returned time is clamped: `max(rtc_secs, VICELL_MIN_UNIX)`.
/// This is a zero-sized type — all state lives in the syscall layer.
pub struct ViTlsClock;

impl TlsClock for ViTlsClock {
    /// Returns `Some(epoch_seconds)`, clamped to the build-time floor.
    ///
    /// Never returns `None` — a missing RTC produces the floor timestamp
    /// rather than disabling expiry checks, which would silently accept
    /// expired certificates.
    fn now() -> Option<u64> {
        let rtc_secs = sys_get_wall_secs();
        Some(rtc_secs.max(VICELL_MIN_UNIX))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_is_sane() {
        // The floor must be at least 2025-06-01 so that modern certs are not
        // immediately expired when the RTC is absent.
        assert!(VICELL_MIN_UNIX >= 1_748_736_000, "floor too early");
    }

    #[test]
    fn now_never_below_floor() {
        // Simulate an epoch-0 RTC by observing that max(0, floor) == floor.
        // We cannot call sys_get_wall_secs in a host test (no kernel), but we
        // can verify the clamp arithmetic directly.
        let rtc_zero: u64 = 0;
        let clamped = rtc_zero.max(VICELL_MIN_UNIX);
        assert_eq!(clamped, VICELL_MIN_UNIX);
    }

    #[test]
    fn now_passes_through_valid_time() {
        // A time well past the floor should pass through unchanged.
        let future_time: u64 = 1_800_000_000; // ~2027
        let clamped = future_time.max(VICELL_MIN_UNIX);
        assert_eq!(clamped, future_time);
    }
}
