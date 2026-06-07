#![no_std]

use types::ViResult;

/// Bus width for MMC/SD data lines.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BusWidth {
    One,
    Four,
    Eight,
}

/// Response type expected after a command.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RespType {
    /// No response (CMD0).
    None,
    /// 48-bit R1 normal response.
    R1,
    /// 48-bit R1b with busy signal on DAT0.
    R1b,
    /// 136-bit R2 (CID/CSD).
    R2,
    /// 48-bit R3 (OCR).
    R3,
    /// 48-bit R6 (RCA publish from SD).
    R6,
    /// 48-bit R7 (interface condition from CMD8).
    R7,
}

/// A single MMC/SD command to send to the controller.
#[derive(Copy, Clone, Debug)]
pub struct MmcCmd {
    pub index: u8,
    pub arg: u32,
    pub resp_type: RespType,
    /// True when this command is accompanied by a data transfer.
    pub has_data: bool,
}

/// 128-bit response register (R2 occupies all 4 words; shorter responses are in word 0).
pub type MmcResponse = [u32; 4];

/// Detected card type after initialization.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CardType {
    /// Embedded MultiMediaCard (CMD1 path).
    Emmc,
    /// SD High Capacity / Extended Capacity (block-addressed).
    SdHc,
    /// SD Standard Capacity (byte-addressed, ≤2 GB).
    SdSc,
}

/// SDHCI host-controller interface (PIO polling mode).
///
/// Implementors: `SdhciController` (kernel). All methods are synchronous.
/// Async DMA support is deferred to a future phase.
///
/// # Invariants
/// - `read_block` / `write_block` operate on exactly `buf.len()` bytes.
///   Callers must ensure `buf.len()` equals the block size set on the controller.
/// - `send_cmd` must not be called while CMD_INHIBIT or DAT_INHIBIT is asserted;
///   callers are responsible for polling readiness first.
pub trait ViMmcHost {
    /// Send `cmd` to the card and return the response registers.
    fn send_cmd(&mut self, cmd: MmcCmd) -> ViResult<MmcResponse>;

    /// PIO-read exactly `buf.len()` bytes from the BUFFER port (after a data command).
    fn read_block(&mut self, buf: &mut [u8]) -> ViResult<()>;

    /// PIO-write exactly `buf.len()` bytes to the BUFFER port (before a data command).
    fn write_block(&mut self, buf: &[u8]) -> ViResult<()>;

    /// Set the controller clock as close to `hz` as possible.
    fn set_clock_hz(&mut self, hz: u32) -> ViResult<()>;

    /// Switch the data bus to the given width (1 / 4 / 8 bits).
    fn set_bus_width(&mut self, width: BusWidth) -> ViResult<()>;

    /// Returns true when a card is physically present in the slot.
    fn card_present(&self) -> bool;
}
