// SDHCI Simplified Specification register offsets (spec v3.00, applies to all SDHCI-compliant controllers).
// All offsets are relative to the controller MMIO base address.

pub const SDHCI_DMA_ADDRESS:    usize = 0x00;
pub const SDHCI_BLOCK_SIZE:     usize = 0x04;
pub const SDHCI_BLOCK_COUNT:    usize = 0x06;
pub const SDHCI_ARGUMENT:       usize = 0x08;
pub const SDHCI_TRANSFER_MODE:  usize = 0x0C;
pub const SDHCI_COMMAND:        usize = 0x0E;
pub const SDHCI_RESPONSE:       usize = 0x10; // [0]=+0x10, [1]=+0x14, [2]=+0x18, [3]=+0x1C
pub const SDHCI_BUFFER:         usize = 0x20;
pub const SDHCI_PRESENT_STATE:  usize = 0x24;
pub const SDHCI_HOST_CONTROL:   usize = 0x28;
pub const SDHCI_POWER_CONTROL:  usize = 0x29;
pub const SDHCI_BLOCK_GAP:      usize = 0x2A;
pub const SDHCI_WAKEUP_CONTROL: usize = 0x2B;
pub const SDHCI_CLOCK_CONTROL:  usize = 0x2C;
pub const SDHCI_TIMEOUT_CONTROL:usize = 0x2E;
pub const SDHCI_SOFT_RESET:     usize = 0x2F;
pub const SDHCI_INT_STATUS:     usize = 0x30;
pub const SDHCI_INT_ENABLE:     usize = 0x34;
pub const SDHCI_SIGNAL_ENABLE:  usize = 0x38;
pub const SDHCI_HOST_CONTROL2:  usize = 0x3E;
pub const SDHCI_CAPABILITIES:   usize = 0x40;
pub const SDHCI_HOST_VERSION:   usize = 0xFE;

// PRESENT_STATE bit masks
pub const PS_CMD_INHIBIT:   u32 = 1 << 0;
pub const PS_DAT_INHIBIT:   u32 = 1 << 1;
pub const PS_DAT_ACTIVE:    u32 = 1 << 2;
pub const PS_CARD_PRESENT:  u32 = 1 << 16;

// INT_STATUS / INT_ENABLE bits
pub const INT_CMD_COMPLETE:     u32 = 1 << 0;
pub const INT_XFER_COMPLETE:    u32 = 1 << 1;
pub const INT_DMA_END:          u32 = 1 << 3;
pub const INT_BUF_WRITE_READY:  u32 = 1 << 4;
pub const INT_BUF_READ_READY:   u32 = 1 << 5;
pub const INT_ERROR:            u32 = 1 << 15;
pub const INT_CMD_TIMEOUT:      u32 = 1 << 16;
pub const INT_CMD_CRC:          u32 = 1 << 17;
pub const INT_CMD_INDEX:        u32 = 1 << 19;
pub const INT_DATA_TIMEOUT:     u32 = 1 << 20;
pub const INT_ALL_NORMAL:       u32 = 0x0000_00FF;
pub const INT_ALL_ERROR:        u32 = 0xFFFF_0000;

// SOFT_RESET bits
pub const RESET_ALL: u8 = 0x01;
pub const RESET_CMD: u8 = 0x02;
pub const RESET_DAT: u8 = 0x04;

// CLOCK_CONTROL bits
pub const CLK_INT_EN:   u16 = 1 << 0;
pub const CLK_INT_STABLE: u16 = 1 << 1;
pub const CLK_SD_EN:    u16 = 1 << 2;

// POWER_CONTROL voltages
pub const PWR_33V: u8 = 0x0E;
pub const PWR_30V: u8 = 0x0C;
pub const PWR_18V: u8 = 0x0A;
pub const PWR_OFF: u8 = 0x00;

// HOST_VERSION spec version field (bits 0..2)
pub const SPEC_V3: u8 = 2;

// TRANSFER_MODE bits
pub const TM_DMA_EN:    u16 = 1 << 0;
pub const TM_BLKCNT_EN: u16 = 1 << 1;
pub const TM_AUTO_CMD23:u16 = 2 << 2; // AUTOCMD23
pub const TM_AUTO_CMD12:u16 = 1 << 2; // AUTOCMD12
pub const TM_DATA_READ: u16 = 1 << 4;
pub const TM_MULTI_BLK: u16 = 1 << 5;

/// Build the 16-bit COMMAND register value.
///
/// `resp_flags` encodes the response type bits (bits 0-1, plus CRC/index check flags).
/// `data` sets the data-present bit (bit 5).
#[inline]
pub fn cmd_reg(index: u8, resp_flags: u16, data: bool) -> u16 {
    let data_bit: u16 = if data { 1 << 5 } else { 0 };
    ((index as u16) << 8) | resp_flags | data_bit
}

// Response flag presets (bits 0-4 of COMMAND register)
pub const RESP_NONE: u16 = 0x00; // no response
pub const RESP_R1:   u16 = 0x1A; // 48-bit, CRC+IDX check
pub const RESP_R1B:  u16 = 0x1B; // 48-bit busy, CRC+IDX check
pub const RESP_R2:   u16 = 0x09; // 136-bit, CRC check only
pub const RESP_R3:   u16 = 0x02; // 48-bit, no check
pub const RESP_R6:   u16 = 0x1A; // same encoding as R1
pub const RESP_R7:   u16 = 0x1A; // same encoding as R1
