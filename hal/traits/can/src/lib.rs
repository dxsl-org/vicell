#![no_std]

/// CAN bus error kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanError {
    /// Transmit buffer full; caller should retry.
    TxFull,
    /// No frame available in the receive buffer.
    RxEmpty,
    /// Controller entered bus-off state (too many errors).
    BusOff,
    /// Arbitration lost; frame was not transmitted.
    ArbitrationLost,
    /// Requested bit rate is not supported.
    InvalidBitrate,
    /// Malformed frame (DLC > 8 or reserved bit violation).
    FrameError,
}

/// A CAN 2.0A (11-bit) or 2.0B (29-bit) data frame.
///
/// Remote frames are not supported in v1.
#[derive(Debug, Clone, Copy)]
pub struct CanFrame {
    /// Frame identifier. Bits 10:0 for standard; bits 28:0 for extended.
    pub id: u32,
    /// `true` → 29-bit extended frame (CAN 2.0B); `false` → 11-bit standard (CAN 2.0A).
    pub extended: bool,
    /// Data Length Code: number of valid bytes in `data` (0–8).
    pub dlc: u8,
    /// Frame payload; only `data[0..dlc]` is meaningful.
    pub data: [u8; 8],
}

impl CanFrame {
    /// Construct a standard (11-bit) frame. `data` is truncated to 8 bytes.
    pub fn new(id: u32, data: &[u8]) -> Self {
        let dlc = data.len().min(8) as u8;
        let mut buf = [0u8; 8];
        buf[..dlc as usize].copy_from_slice(&data[..dlc as usize]);
        Self { id: id & 0x7FF, extended: false, dlc, data: buf }
    }

    /// Construct an extended (29-bit) frame. `data` is truncated to 8 bytes.
    pub fn new_ext(id: u32, data: &[u8]) -> Self {
        let dlc = data.len().min(8) as u8;
        let mut buf = [0u8; 8];
        buf[..dlc as usize].copy_from_slice(&data[..dlc as usize]);
        Self { id: id & 0x1FFF_FFFF, extended: true, dlc, data: buf }
    }
}

/// CAN bus controller trait.
///
/// # Contract
/// - All operations are synchronous.
/// - Bit rate must be configured before any frame is sent or received.
/// - After `BusOff`, the implementation must be reset before further use.
pub trait ViCan {
    type Error: core::fmt::Debug;

    /// Configure bit timing. Supported rates: 125, 250, 500, 1000 kbps.
    /// Returns `InvalidBitrate` for any other value.
    fn configure(&mut self, kbps: u32) -> Result<(), Self::Error>;

    /// Transmit a CAN frame. Returns `TxFull` if the transmit buffer is full.
    fn send_frame(&mut self, frame: &CanFrame) -> Result<(), Self::Error>;

    /// Receive the next available frame. Returns `RxEmpty` if no frame is pending.
    fn recv_frame(&mut self) -> Result<CanFrame, Self::Error>;
}
