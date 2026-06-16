#![no_std]
#![forbid(unsafe_code)]

//! Bit-bang PWM over a GPIO pin.
//!
//! Generic over any `G: ViGpio`. Concrete use:
//! `BitBangPwm::<Pl061Gpio>::new(gpio)` — caller opens the GPIO device first.
//!
//! Timing model: counter-based (no wall-clock dependency). The caller drives
//! `tick()` in a tight pinned-poll loop (spec 13-peripherals §6). Each call
//! increments an internal counter; pins are toggled when counter thresholds
//! cross the configured duty/period boundary.
//!
//! `set_frequency(channel, hz)` converts `hz` to a period in tick-counts
//! assuming the caller drives `tick()` at `ASSUMED_TICKS_PER_SEC` iterations/sec.
//! On QEMU TCG the actual loop rate varies; `hz` controls the duty ratio only —
//! not the precise output frequency.

use hal_gpio::{PinDir, ViGpio};
use hal_pwm::{PwmError, ViPwm};

/// Assumed tick() calls per second for frequency→period conversion.
/// This only affects how Hz maps to counter counts, not output correctness.
const ASSUMED_TICKS_PER_SEC: u32 = 10_000;

/// Maximum channels (one per GPIO pin).
const MAX_CHANNELS: usize = 8;

/// Per-channel waveform state.
struct Channel {
    pin: u8,
    /// Total counter count per waveform period.
    period_ticks: u32,
    /// Counter count for the HIGH portion of each period.
    high_ticks: u32,
    /// Current position within the period (0..period_ticks).
    counter: u32,
    /// Current output level.
    high: bool,
    enabled: bool,
}

impl Channel {
    const fn idle(pin: u8) -> Self {
        Self { pin, period_ticks: 1, high_ticks: 0, counter: 0, high: false, enabled: false }
    }
}

/// Bit-bang PWM master backed by a `ViGpio` implementation.
pub struct BitBangPwm<G: ViGpio> {
    gpio: G,
    channels: [Channel; MAX_CHANNELS],
}

impl<G: ViGpio> BitBangPwm<G> {
    /// Take ownership of `gpio` and prepare it as a PWM source.
    pub fn new(gpio: G) -> Self {
        Self {
            gpio,
            channels: [
                Channel::idle(0),
                Channel::idle(1),
                Channel::idle(2),
                Channel::idle(3),
                Channel::idle(4),
                Channel::idle(5),
                Channel::idle(6),
                Channel::idle(7),
            ],
        }
    }

    /// Release the underlying GPIO resource back to the caller.
    pub fn into_gpio(self) -> G {
        self.gpio
    }
}

impl<G: ViGpio> ViPwm for BitBangPwm<G> {
    type Error = PwmError;

    fn set_frequency(&mut self, channel: u8, hz: u32) -> Result<(), PwmError> {
        if channel as usize >= MAX_CHANNELS {
            return Err(PwmError::InvalidChannel);
        }
        if hz == 0 || hz > 50_000 {
            return Err(PwmError::InvalidFrequency);
        }
        let ch = &mut self.channels[channel as usize];
        let period = (ASSUMED_TICKS_PER_SEC / hz).max(1);
        // Recompute high_ticks to preserve duty ratio across frequency changes.
        let duty = ch.high_ticks.checked_mul(1000)
            .and_then(|n| n.checked_div(ch.period_ticks))
            .unwrap_or(0);
        ch.period_ticks = period;
        ch.high_ticks = (period * duty / 1000).max(if duty > 0 { 1 } else { 0 });
        Ok(())
    }

    fn set_duty(&mut self, channel: u8, duty_per_mille: u16) -> Result<(), PwmError> {
        if channel as usize >= MAX_CHANNELS {
            return Err(PwmError::InvalidChannel);
        }
        if duty_per_mille > 1000 {
            return Err(PwmError::InvalidDuty);
        }
        let ch = &mut self.channels[channel as usize];
        ch.high_ticks = (ch.period_ticks * duty_per_mille as u32 / 1000)
            .max(if duty_per_mille > 0 { 1 } else { 0 });
        Ok(())
    }

    fn enable(&mut self, channel: u8) -> Result<(), PwmError> {
        if channel as usize >= MAX_CHANNELS {
            return Err(PwmError::InvalidChannel);
        }
        let pin = self.channels[channel as usize].pin;
        self.gpio.set_direction(pin, PinDir::Output).map_err(|_| PwmError::InvalidChannel)?;
        self.channels[channel as usize].counter = 0;
        self.channels[channel as usize].enabled = true;
        Ok(())
    }

    fn disable(&mut self, channel: u8) -> Result<(), PwmError> {
        if channel as usize >= MAX_CHANNELS {
            return Err(PwmError::InvalidChannel);
        }
        let ch = &mut self.channels[channel as usize];
        ch.enabled = false;
        ch.high = false;
        let pin = ch.pin;
        // Best-effort drive LOW; ignore if GPIO already released.
        let _ = self.gpio.write_pin(pin, false);
        Ok(())
    }

    fn tick(&mut self) {
        for ch in self.channels.iter_mut() {
            if !ch.enabled {
                continue;
            }
            ch.counter = ch.counter.wrapping_add(1);
            if ch.counter >= ch.period_ticks {
                ch.counter = 0;
            }
            let should_be_high = ch.counter < ch.high_ticks;
            if should_be_high != ch.high {
                ch.high = should_be_high;
                // Best-effort; ignore GPIO errors in hot loop.
                let _ = self.gpio.write_pin(ch.pin, should_be_high);
            }
        }
    }
}
