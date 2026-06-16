#![no_std]
#![forbid(unsafe_code)]

//! Simulation ADC — returns synthetic ramp values, no MMIO required.
//!
//! Useful for testing robot firmware loops on QEMU without real analog hardware.
//! Each channel follows an independent triangle-wave ramp (0 → 4095 → 0, cycling)
//! with optional deterministic noise.
//!
//! Call `step()` once per sample period to advance the simulation clock.
//! Each `read_raw(channel)` returns the current value for that channel.

use hal_adc::{AdcError, ViAdc};

const MAX_CHANNELS: usize = 8;
const MAX_VALUE: u16 = 4095;

/// Per-channel simulation parameters.
#[derive(Clone, Copy)]
pub struct SimChannel {
    /// Number of `step()` calls for a full 0→4095→0 triangle cycle.
    pub ramp_period_steps: u32,
    /// Add deterministic noise with amplitude ±(1 << noise_bits).
    /// Set to 0 for a clean ramp.
    pub noise_bits: u8,
}

impl SimChannel {
    pub const fn new(ramp_period_steps: u32, noise_bits: u8) -> Self {
        Self { ramp_period_steps, noise_bits }
    }
}

/// Simulation ADC with up to 8 independently configured channels.
pub struct SimAdc {
    channels: [Option<SimChannel>; MAX_CHANNELS],
    /// Global step counter; incremented by `step()`.
    tick: u32,
}

impl SimAdc {
    pub fn new() -> Self {
        Self { channels: [None; MAX_CHANNELS], tick: 0 }
    }

    /// Configure a channel. Replaces any prior configuration.
    pub fn configure(&mut self, channel: u8, cfg: SimChannel) {
        if (channel as usize) < MAX_CHANNELS {
            self.channels[channel as usize] = Some(cfg);
        }
    }

    /// Advance the simulation clock by one sample period.
    /// Call this before `read_raw` in each polling iteration.
    pub fn step(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    fn compute_raw(&self, cfg: &SimChannel) -> u16 {
        let period = cfg.ramp_period_steps.max(2);
        // Half-period = rising half.
        let half = period / 2;
        let phase = self.tick % period;
        let raw = if phase < half {
            // Rising: 0 → MAX_VALUE
            (MAX_VALUE as u32 * phase / half.max(1)) as u16
        } else {
            // Falling: MAX_VALUE → 0
            let down = phase - half;
            let down_half = period - half;
            MAX_VALUE - (MAX_VALUE as u32 * down / down_half.max(1)) as u16
        };
        // Deterministic noise via multiplicative hash (no RNG needed).
        if cfg.noise_bits == 0 {
            return raw;
        }
        let noise_amp = 1u16 << cfg.noise_bits;
        let hash = self.tick.wrapping_mul(2_654_435_761) >> 20;
        let noise = (hash as u16) & (noise_amp.saturating_sub(1));
        raw.saturating_add(noise).min(MAX_VALUE)
    }
}

impl Default for SimAdc {
    fn default() -> Self {
        Self::new()
    }
}

impl ViAdc for SimAdc {
    type Error = AdcError;

    fn read_raw(&mut self, channel: u8) -> Result<u16, AdcError> {
        if (channel as usize) >= MAX_CHANNELS {
            return Err(AdcError::InvalidChannel);
        }
        match &self.channels[channel as usize] {
            None => Err(AdcError::InvalidChannel),
            Some(cfg) => Ok(self.compute_raw(cfg)),
        }
    }

    fn max_value(&self) -> u16 {
        MAX_VALUE
    }

    fn num_channels(&self) -> u8 {
        MAX_CHANNELS as u8
    }
}
