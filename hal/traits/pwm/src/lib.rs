#![no_std]

/// PWM channel error kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PwmError {
    /// Channel index out of range.
    InvalidChannel,
    /// Frequency is zero or exceeds implementation maximum.
    InvalidFrequency,
    /// Duty cycle exceeds 1000 per mille.
    InvalidDuty,
    /// Operation on a channel that has not been enabled.
    NotEnabled,
}

/// Pulse-width modulation output trait.
///
/// # Contract
/// - `channel` is a zero-based index. Implementations define the maximum number of channels.
/// - `duty_per_mille` is the on-time fraction × 1000: 0 = always LOW, 1000 = always HIGH,
///   500 = 50% duty cycle.
/// - `tick()` MUST be called continuously in a tight loop to advance the waveform state machine.
///   Callers that stop calling `tick()` will observe a stuck output level.
/// - After `disable(channel)`, the pin is driven LOW.
pub trait ViPwm {
    type Error: core::fmt::Debug;

    /// Configure waveform frequency (Hz) for the given channel.
    /// Valid range: 1–50 000 Hz (implementation may restrict further).
    fn set_frequency(&mut self, channel: u8, hz: u32) -> Result<(), Self::Error>;

    /// Configure duty cycle in per mille (0–1000) for the given channel.
    fn set_duty(&mut self, channel: u8, duty_per_mille: u16) -> Result<(), Self::Error>;

    /// Start waveform generation on `channel` at the configured frequency and duty.
    fn enable(&mut self, channel: u8) -> Result<(), Self::Error>;

    /// Stop waveform generation; the pin is driven LOW.
    fn disable(&mut self, channel: u8) -> Result<(), Self::Error>;

    /// Advance the PWM state machine by one step.
    ///
    /// Call this in a tight pinned-poll loop (see spec 13-peripherals.md §6).
    /// Each call may toggle output pins for any enabled channel whose deadline has elapsed.
    fn tick(&mut self);
}
