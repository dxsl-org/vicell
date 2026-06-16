#![no_std]

use types::ViResult;

/// Pin direction: input or output.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PinDir {
    Input,
    Output,
}

/// Edge that triggers an interrupt.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Edge {
    Rising,
    Falling,
    Both,
}

/// GPIO controller trait for Driver Cells.
///
/// Implemented by `cells/drivers/gpio-pl061` (and future per-chip impls).
/// `#![forbid(unsafe_code)]` Cells use this via the safe `MmioRegion` abstraction.
///
/// # Invariants
/// - Pin indices are 0-based; out-of-range → `Err(ViError::InvalidInput)`.
/// - `write_pin` on an input pin → `Err(ViError::InvalidInput)`.
pub trait ViGpio {
    /// Configure `pin` as input or output.
    fn set_direction(&mut self, pin: u8, dir: PinDir) -> ViResult<()>;

    /// Read the current logic level of `pin`.
    fn read_pin(&self, pin: u8) -> ViResult<bool>;

    /// Write a logic level to an output `pin`.
    fn write_pin(&mut self, pin: u8, high: bool) -> ViResult<()>;

    /// Enable edge-triggered interrupt on `pin`.
    ///
    /// The implementation routes the interrupt to the calling Cell's waker;
    /// the Cell polls via its async runtime.
    fn enable_edge_irq(&mut self, pin: u8, edge: Edge) -> ViResult<()>;

    /// Disable any interrupt previously enabled on `pin`.
    fn disable_irq(&mut self, pin: u8) -> ViResult<()>;
}
