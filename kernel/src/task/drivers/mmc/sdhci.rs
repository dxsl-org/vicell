use super::regs::*;
use hal_traits_mmc::{BusWidth, MmcCmd, MmcResponse, RespType, ViMmcHost};
use types::{ViError, ViResult};

/// Polling timeout for CMD_COMPLETE and DAT transfers (~500 ms at 1 iteration/µs).
const POLL_TIMEOUT_US: u32 = 500_000;

/// SDHCI host controller — PIO polling mode, no DMA, no interrupts.
///
/// The `base` field is the kernel-mapped virtual address of the SDHCI register block.
/// All MMIO accesses are `read_volatile` / `write_volatile` to prevent optimisation.
pub struct SdhciController {
    base: usize,
    /// SDHC = true (block-addressed); SDSC = false (byte-addressed).
    pub is_sdhc: bool,
    /// SDHCI spec version read from HOST_VERSION[2:0]; affects clock divider encoding.
    spec_ver: u8,
}

impl SdhciController {
    /// Construct a controller from `base`.
    ///
    /// # Safety
    /// `base` must be a valid kernel-mapped MMIO address for the SDHCI register block.
    /// The address must remain valid for the lifetime of `Self`.
    pub unsafe fn new(base: usize) -> Self {
        let mut c = Self { base, is_sdhc: false, spec_ver: 0 };
        // SAFETY: base is the validated MMIO address passed by the caller.
        c.spec_ver = (c.read16(SDHCI_HOST_VERSION) & 0xFF) as u8;
        c
    }

    // --- volatile MMIO helpers ---

    #[inline]
    fn read32(&self, off: usize) -> u32 {
        // SAFETY: base + off is within the SDHCI MMIO block mapped by the kernel.
        unsafe { core::ptr::read_volatile((self.base + off) as *const u32) }
    }
    #[inline]
    fn read16(&self, off: usize) -> u16 {
        // SAFETY: same as read32.
        unsafe { core::ptr::read_volatile((self.base + off) as *const u16) }
    }
    #[inline]
    fn write32(&mut self, off: usize, v: u32) {
        // SAFETY: same as read32.
        unsafe { core::ptr::write_volatile((self.base + off) as *mut u32, v) }
    }
    #[inline]
    fn write16(&mut self, off: usize, v: u16) {
        // SAFETY: same as read32.
        unsafe { core::ptr::write_volatile((self.base + off) as *mut u16, v) }
    }
    #[inline]
    fn write8(&mut self, off: usize, v: u8) {
        // SAFETY: same as read32.
        unsafe { core::ptr::write_volatile((self.base + off) as *mut u8, v) }
    }

    /// Spin until `(read32(off) & mask) == 0`, or return `Err(Timeout)`.
    fn poll_clear(&self, off: usize, mask: u32, timeout_us: u32) -> ViResult<()> {
        let mut i = 0u32;
        while self.read32(off) & mask != 0 {
            if i >= timeout_us {
                return Err(ViError::WouldBlock);
            }
            i += 1;
            // Single iteration ≈ 1 µs on a 1 GHz core with one memory-mapped read.
            core::hint::spin_loop();
        }
        Ok(())
    }

    /// Spin until `(read32(off) & mask) != 0`, or return `Err(Timeout)`.
    fn poll_set(&self, off: usize, mask: u32, timeout_us: u32) -> ViResult<()> {
        let mut i = 0u32;
        while self.read32(off) & mask == 0 {
            if i >= timeout_us {
                return Err(ViError::WouldBlock);
            }
            i += 1;
            core::hint::spin_loop();
        }
        Ok(())
    }

    /// Reset the controller (all lines).
    pub fn reset_all(&mut self) -> ViResult<()> {
        self.write8(SDHCI_SOFT_RESET, RESET_ALL);
        self.poll_clear(SDHCI_SOFT_RESET as usize, RESET_ALL as u32, POLL_TIMEOUT_US)?;
        Ok(())
    }

    /// Enable 3.3 V power to the card slot.
    pub fn power_on(&mut self) {
        self.write8(SDHCI_POWER_CONTROL, PWR_33V);
    }

    /// Set the SD clock to the requested divider.
    ///
    /// Uses spec-v3 10-bit divider encoding when `spec_ver >= SPEC_V3`.
    fn set_clock_div(&mut self, div: u16) {
        // Disable SD clock and internal clock first.
        self.write16(SDHCI_CLOCK_CONTROL, 0);

        let clk = if self.spec_ver >= SPEC_V3 {
            // 10-bit divider: bits[7:0] in bits[15:8], bits[9:8] in bits[7:6].
            let lo = (div & 0xFF) as u16;
            let hi = ((div >> 8) & 0x03) as u16;
            (lo << 8) | (hi << 6) | CLK_INT_EN
        } else {
            // 8-bit divider (spec v1/v2): bits[7:0] in bits[15:8].
            ((div & 0xFF) << 8) | CLK_INT_EN
        };

        self.write16(SDHCI_CLOCK_CONTROL, clk);
        // Wait for internal clock to stabilise.
        let _ = self.poll_set(
            SDHCI_CLOCK_CONTROL as usize,
            CLK_INT_STABLE as u32,
            POLL_TIMEOUT_US,
        );
        // Enable SD clock to card.
        self.write16(SDHCI_CLOCK_CONTROL, clk | CLK_SD_EN);
    }

    /// Read the INT_STATUS register and clear the given bits (w1c).
    fn clear_int(&mut self, bits: u32) {
        self.write32(SDHCI_INT_STATUS, bits);
    }

    /// Wait for CMD_INHIBIT and DAT_INHIBIT to clear before issuing a command.
    fn wait_cmd_ready(&self, needs_dat: bool) -> ViResult<()> {
        let mask = if needs_dat {
            PS_CMD_INHIBIT | PS_DAT_INHIBIT
        } else {
            PS_CMD_INHIBIT
        };
        self.poll_clear(SDHCI_PRESENT_STATE, mask, POLL_TIMEOUT_US)
    }
}

impl Drop for SdhciController {
    fn drop(&mut self) {
        // Power off the card slot on controller teardown.
        self.write8(SDHCI_POWER_CONTROL, PWR_OFF);
        self.write16(SDHCI_CLOCK_CONTROL, 0);
    }
}

impl ViMmcHost for SdhciController {
    fn send_cmd(&mut self, cmd: MmcCmd) -> ViResult<MmcResponse> {
        self.wait_cmd_ready(cmd.has_data)?;

        // Unmask normal+error interrupts in INT_ENABLE (no CPU IRQ — polling only).
        self.write32(SDHCI_INT_ENABLE, INT_ALL_NORMAL | INT_ALL_ERROR);
        self.write32(SDHCI_SIGNAL_ENABLE, 0); // no CPU interrupt

        // Build response flag bits for the COMMAND register.
        let resp_flags: u16 = match cmd.resp_type {
            RespType::None => RESP_NONE,
            RespType::R1   => RESP_R1,
            RespType::R1b  => RESP_R1B,
            RespType::R2   => RESP_R2,
            RespType::R3   => RESP_R3,
            RespType::R6   => RESP_R6,
            RespType::R7   => RESP_R7,
        };

        self.write32(SDHCI_ARGUMENT, cmd.arg);
        // Writing COMMAND fires the command to the card.
        self.write16(SDHCI_COMMAND, cmd_reg(cmd.index, resp_flags, cmd.has_data));

        // Wait for CMD_COMPLETE (bit 0) or an error.
        self.poll_set(SDHCI_INT_STATUS, INT_CMD_COMPLETE | INT_ERROR, POLL_TIMEOUT_US)?;

        let status = self.read32(SDHCI_INT_STATUS);
        self.clear_int(INT_CMD_COMPLETE | INT_ALL_ERROR);

        if status & INT_ERROR != 0 {
            log::warn!("[sdhci] cmd{} error, INT_STATUS=0x{:08x}", cmd.index, status);
            return Err(ViError::IO);
        }

        // Read response registers.
        let r = [
            self.read32(SDHCI_RESPONSE),
            self.read32(SDHCI_RESPONSE + 4),
            self.read32(SDHCI_RESPONSE + 8),
            self.read32(SDHCI_RESPONSE + 12),
        ];
        Ok(r)
    }

    fn read_block(&mut self, buf: &mut [u8]) -> ViResult<()> {
        // Caller must pass a 512-byte, 4-byte-aligned buffer (one SDHCI block).
        if buf.len() != 512 {
            return Err(ViError::InvalidArgument);
        }

        // Wait for BUFFER_READ_READY (data available in FIFO).
        self.poll_set(SDHCI_INT_STATUS, INT_BUF_READ_READY, POLL_TIMEOUT_US)?;
        self.clear_int(INT_BUF_READ_READY);

        // Read 4 bytes at a time from the BUFFER port.
        let chunks = buf.len() / 4;
        for i in 0..chunks {
            let word = self.read32(SDHCI_BUFFER);
            let off = i * 4;
            buf[off]     = (word & 0xFF) as u8;
            buf[off + 1] = ((word >> 8) & 0xFF) as u8;
            buf[off + 2] = ((word >> 16) & 0xFF) as u8;
            buf[off + 3] = ((word >> 24) & 0xFF) as u8;
        }

        // Wait for TRANSFER_COMPLETE.
        self.poll_set(SDHCI_INT_STATUS, INT_XFER_COMPLETE, POLL_TIMEOUT_US)?;
        self.clear_int(INT_XFER_COMPLETE);
        Ok(())
    }

    fn write_block(&mut self, buf: &[u8]) -> ViResult<()> {
        if buf.len() != 512 {
            return Err(ViError::InvalidArgument);
        }

        // Wait for BUFFER_WRITE_READY (FIFO has space).
        self.poll_set(SDHCI_INT_STATUS, INT_BUF_WRITE_READY, POLL_TIMEOUT_US)?;
        self.clear_int(INT_BUF_WRITE_READY);

        let chunks = buf.len() / 4;
        for i in 0..chunks {
            let off = i * 4;
            let word = (buf[off] as u32)
                | ((buf[off + 1] as u32) << 8)
                | ((buf[off + 2] as u32) << 16)
                | ((buf[off + 3] as u32) << 24);
            self.write32(SDHCI_BUFFER, word);
        }

        self.poll_set(SDHCI_INT_STATUS, INT_XFER_COMPLETE, POLL_TIMEOUT_US)?;
        self.clear_int(INT_XFER_COMPLETE);
        Ok(())
    }

    fn set_clock_hz(&mut self, hz: u32) -> ViResult<()> {
        // Base clock frequency (typically 200 MHz on Arasan, 50 MHz on some others).
        // We assume 200 MHz; the divider is rounded up to the nearest power-of-2 (spec v1/v2)
        // or any value (spec v3). For boot-time use we target either 400 kHz (ID) or 25 MHz (DS).
        const BASE_HZ: u32 = 200_000_000;
        let div = if hz == 0 { 0 } else { (BASE_HZ / hz / 2).max(1) as u16 };
        self.set_clock_div(div);
        Ok(())
    }

    fn set_bus_width(&mut self, width: BusWidth) -> ViResult<()> {
        let mut hc = self.read32(SDHCI_HOST_CONTROL) as u8;
        hc &= !0x26; // clear 4-bit (bit1) and 8-bit (bit5) fields
        match width {
            BusWidth::One  => {}
            BusWidth::Four => hc |= 1 << 1,
            BusWidth::Eight => hc |= 1 << 5,
        }
        self.write8(SDHCI_HOST_CONTROL, hc);
        Ok(())
    }

    fn card_present(&self) -> bool {
        self.read32(SDHCI_PRESENT_STATE) & PS_CARD_PRESENT != 0
    }
}

impl SdhciController {
    /// Configure BLOCK_SIZE, BLOCK_COUNT, and TRANSFER_MODE for an upcoming data command.
    pub(super) fn setup_data_transfer(&mut self, block_size: u16, block_count: u16, transfer_mode: u16) {
        self.write16(SDHCI_BLOCK_SIZE, block_size);
        self.write16(SDHCI_BLOCK_COUNT, block_count);
        self.write16(SDHCI_TRANSFER_MODE, transfer_mode);
    }
}
