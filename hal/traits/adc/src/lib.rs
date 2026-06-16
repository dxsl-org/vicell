#![no_std]

/// ADC conversion error kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdcError {
    /// Channel index out of range.
    InvalidChannel,
    /// Hardware did not complete conversion in time.
    ConversionTimeout,
    /// Hardware fault or peripheral error.
    HardwareError,
}

/// Analog-to-digital converter trait.
///
/// # Contract
/// - `channel` is a zero-based index. Implementations define the number of channels.
/// - Raw values are in the range `[0, max_value()]` inclusive.
/// - `to_millivolts` is a provided default method; implementations may override it
///   for higher precision or non-linear calibration.
pub trait ViAdc {
    type Error: core::fmt::Debug;

    /// Read a raw ADC sample from the given channel.
    fn read_raw(&mut self, channel: u8) -> Result<u16, Self::Error>;

    /// Maximum raw value (e.g., 4095 for a 12-bit ADC).
    fn max_value(&self) -> u16;

    /// Number of available channels.
    fn num_channels(&self) -> u8;

    /// Convert a raw reading to millivolts given the reference supply voltage.
    ///
    /// Uses linear scaling: `mV = supply_mv × raw / max_value()`.
    fn to_millivolts(&self, raw: u16, supply_mv: u32) -> u32 {
        (supply_mv as u64 * raw as u64 / self.max_value() as u64) as u32
    }
}
