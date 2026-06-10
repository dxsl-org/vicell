#![no_std]

use types::{HalResult, ViResult};

/// A simple Serial/UART interface
pub trait SerialPort {
    /// Initialize the serial port (baud rate, etc.)
    fn init(&mut self) -> HalResult<()>;

    /// Write a single byte
    fn send(&mut self, data: u8) -> HalResult<()>;

    /// Read a single byte (blocking or polling)
    fn receive(&mut self) -> HalResult<u8>;
}

/// Helper to write strings
pub trait SerialWrite: SerialPort {
    fn write_str(&mut self, s: &str) -> HalResult<()> {
        for byte in s.bytes() {
            self.send(byte)?;
        }
        Ok(())
    }
}

impl<T: SerialPort> SerialWrite for T {}

/// UART baud rate (common values; driver validates against hardware limits).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BaudRate {
    B9600   = 9600,
    B115200 = 115200,
    B1000000 = 1_000_000,
}

/// Data-frame configuration (parity, stop bits).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct UartConfig {
    pub baud:      BaudRate,
    /// Data bits per frame (5–8; driver enforces).
    pub data_bits: u8,
    /// Stop bits (1 or 2).
    pub stop_bits: u8,
    pub parity:    Parity,
}

/// Parity mode.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Parity { None, Even, Odd }

/// Extended UART trait for Driver Cells (adds runtime reconfiguration).
///
/// Implementors: `cells/drivers/uart-pl011` on QEMU ARM virt.
pub trait ViUart: SerialPort {
    /// Reconfigure the UART.  Must be called before `send`/`receive`.
    fn configure(&mut self, cfg: UartConfig) -> ViResult<()>;

    /// Returns `true` if the receive FIFO / buffer is non-empty.
    fn rx_ready(&self) -> bool;

    /// Returns `true` if the transmit FIFO / buffer has room.
    fn tx_ready(&self) -> bool;
}
