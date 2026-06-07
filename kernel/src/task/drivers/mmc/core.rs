use super::regs::*;
use super::sdhci::SdhciController;
use hal_traits_mmc::{CardType, MmcCmd, RespType, ViMmcHost};
use types::{ViError, ViResult};

/// Card identification and geometry, returned by [`MmcCore::init_card`].
pub struct CardInfo {
    pub card_type: CardType,
    pub rca: u16,
    pub sector_count: u64,
    /// True for SDHC/SDXC/eMMC (block-addressed); false for SDSC (byte-addressed).
    pub is_block_addressed: bool,
}

/// MMC protocol layer. Owns the SDHCI controller and runs the card-init state machine.
pub struct MmcCore {
    pub(super) host: SdhciController,
}

impl MmcCore {
    /// Create an `MmcCore` from a raw SDHCI MMIO base address.
    ///
    /// # Safety
    /// `sdhci_base` must be a valid kernel-mapped MMIO address for the SDHCI register block.
    pub unsafe fn new(sdhci_base: usize) -> Self {
        // SAFETY: forwarded from caller contract.
        Self { host: SdhciController::new(sdhci_base) }
    }

    /// Run the full card initialization sequence.
    ///
    /// Returns [`CardInfo`] describing the detected card type, RCA, and sector count.
    /// On success the card is in Transfer state, clocked at ~25 MHz, 1-bit bus.
    pub fn init_card(&mut self) -> ViResult<CardInfo> {
        // Step 1 — hardware reset, power on, 400 kHz identification clock.
        self.host.reset_all()?;
        self.host.power_on();
        self.host.set_clock_hz(400_000)?;

        // Step 2 — CMD0: GO_IDLE
        self.cmd0_go_idle()?;

        // Step 3 — probe card type via CMD8
        let is_sd_v2 = self.cmd8_send_if_cond()?;

        // Step 4 — operating-condition negotiation
        let (ocr, card_type) = if !is_sd_v2 {
            // Try eMMC CMD1 first; fall back to SD v1 ACMD41 on failure.
            match self.cmd1_emmc_ocr_loop() {
                Ok(ocr) => (ocr, CardType::Emmc),
                Err(_) => {
                    self.cmd0_go_idle()?; // re-idle before SD path
                    let ocr = self.acmd41_sd_ocr_loop(false)?;
                    (ocr, if ocr & (1 << 30) != 0 { CardType::SdHc } else { CardType::SdSc })
                }
            }
        } else {
            let ocr = self.acmd41_sd_ocr_loop(true)?;
            (ocr, if ocr & (1 << 30) != 0 { CardType::SdHc } else { CardType::SdSc })
        };

        let is_block_addressed = matches!(card_type, CardType::Emmc | CardType::SdHc)
            || (ocr & (1 << 30) != 0);

        // Step 5 — CMD2 (ALL_SEND_CID), CMD3 (SET_RELATIVE_ADDR), CMD7 (SELECT)
        self.cmd2_all_send_cid()?;
        let rca = match card_type {
            CardType::Emmc => {
                // eMMC: host assigns RCA 1.
                self.cmd3_set_rca(1)?
            }
            _ => {
                // SD: card proposes RCA; arg=0 asks card to publish its RCA.
                self.cmd3_set_rca(0)?
            }
        };
        self.cmd7_select(rca)?;

        // Step 6 — switch to 25 MHz data clock.
        self.host.set_clock_hz(25_000_000)?;

        // Step 7 — read sector count.
        let sector_count = match card_type {
            CardType::Emmc => self.emmc_read_ext_csd()?,
            _ => self.sd_read_csd(rca)?,
        };

        Ok(CardInfo { card_type, rca, sector_count, is_block_addressed })
    }

    // --- private command helpers ---

    fn cmd0_go_idle(&mut self) -> ViResult<()> {
        let cmd = MmcCmd { index: 0, arg: 0, resp_type: RespType::None, has_data: false };
        // CMD0 has no response; ignore the (zeroed) return value.
        let _ = self.host.send_cmd(cmd);
        Ok(())
    }

    /// Send CMD8. Returns `true` if the card is SD v2+ (echoed pattern matches).
    fn cmd8_send_if_cond(&mut self) -> ViResult<bool> {
        let cmd = MmcCmd {
            index: 8,
            arg: 0x0000_01AA, // voltage range 2.7–3.6 V, check pattern 0xAA
            resp_type: RespType::R7,
            has_data: false,
        };
        match self.host.send_cmd(cmd) {
            Ok(r) => Ok(r[0] & 0xFF == 0xAA),
            Err(_) => Ok(false), // illegal-command error = SD v1 or eMMC
        }
    }

    /// eMMC operating-condition loop (CMD1, up to 500 ms).
    fn cmd1_emmc_ocr_loop(&mut self) -> ViResult<u32> {
        for _ in 0..5000 {
            let cmd = MmcCmd {
                index: 1,
                arg: 0x40FF_8080, // sector-mode + voltage bits
                resp_type: RespType::R3,
                has_data: false,
            };
            let r = self.host.send_cmd(cmd)?;
            let ocr = r[0];
            if ocr & (1 << 31) != 0 {
                return Ok(ocr);
            }
            // ~100 µs busy-wait between retries.
            for _ in 0..100 {
                core::hint::spin_loop();
            }
        }
        Err(ViError::WouldBlock)
    }

    /// SD operating-condition loop (ACMD41, up to 500 ms). `hcs=true` for v2+.
    fn acmd41_sd_ocr_loop(&mut self, hcs: bool) -> ViResult<u32> {
        let hcs_bit: u32 = if hcs { 1 << 30 } else { 0 };
        for _ in 0..5000 {
            self.cmd55_app_cmd(0)?;
            let cmd = MmcCmd {
                index: 41,
                arg: hcs_bit | 0x00FF_8000,
                resp_type: RespType::R3,
                has_data: false,
            };
            let r = self.host.send_cmd(cmd)?;
            let ocr = r[0];
            if ocr & (1 << 31) != 0 {
                return Ok(ocr);
            }
            for _ in 0..100 {
                core::hint::spin_loop();
            }
        }
        Err(ViError::WouldBlock)
    }

    fn cmd55_app_cmd(&mut self, rca: u16) -> ViResult<()> {
        let cmd = MmcCmd {
            index: 55,
            arg: (rca as u32) << 16,
            resp_type: RespType::R1,
            has_data: false,
        };
        self.host.send_cmd(cmd)?;
        Ok(())
    }

    fn cmd2_all_send_cid(&mut self) -> ViResult<()> {
        let cmd = MmcCmd { index: 2, arg: 0, resp_type: RespType::R2, has_data: false };
        self.host.send_cmd(cmd)?;
        Ok(())
    }

    /// CMD3 — set/publish RCA.
    /// For eMMC, arg = proposed RCA (non-zero). Returns the active RCA.
    /// For SD, arg = 0; card publishes its RCA in R6 bits[31:16].
    fn cmd3_set_rca(&mut self, proposed: u16) -> ViResult<u16> {
        let cmd = MmcCmd {
            index: 3,
            arg: (proposed as u32) << 16,
            resp_type: RespType::R6,
            has_data: false,
        };
        let r = self.host.send_cmd(cmd)?;
        let rca = if proposed != 0 {
            proposed
        } else {
            (r[0] >> 16) as u16
        };
        Ok(rca)
    }

    fn cmd7_select(&mut self, rca: u16) -> ViResult<()> {
        let cmd = MmcCmd {
            index: 7,
            arg: (rca as u32) << 16,
            resp_type: RespType::R1b,
            has_data: false,
        };
        self.host.send_cmd(cmd)?;
        Ok(())
    }

    /// Read eMMC EXT_CSD (512 bytes) and extract the sector count from bytes [215:212].
    fn emmc_read_ext_csd(&mut self) -> ViResult<u64> {
        // CMD23 (SET_BLOCK_COUNT) before CMD8 in Transfer state.
        let cmd23 = MmcCmd { index: 23, arg: 1, resp_type: RespType::R1, has_data: false };
        self.host.send_cmd(cmd23)?;

        // Set BLOCK_SIZE=512, BLOCK_COUNT=1, TRANSFER_MODE=read-single.
        self.host.setup_data_transfer(0x0200, 1, TM_DATA_READ);

        let cmd8 = MmcCmd { index: 8, arg: 0, resp_type: RespType::R1, has_data: true };
        self.host.send_cmd(cmd8)?;

        let mut ext_csd = [0u8; 512];
        self.host.read_block(&mut ext_csd)?;

        // Sector count at EXT_CSD bytes 215..212 (little-endian u32).
        let count = u32::from_le_bytes([ext_csd[212], ext_csd[213], ext_csd[214], ext_csd[215]]);
        Ok(count as u64)
    }

    /// Read SD CSD (CMD9) and decode sector count (CSD v1 and v2).
    fn sd_read_csd(&mut self, rca: u16) -> ViResult<u64> {
        let cmd9 = MmcCmd {
            index: 9,
            arg: (rca as u32) << 16,
            resp_type: RespType::R2,
            has_data: false,
        };
        let r = self.host.send_cmd(cmd9)?;

        // SDHCI stores R2 response[127:8] in RESP[119:0] with RESP[127:120]=0.
        // Mapping: r[n][b] = CSD[n*32 + b + 8] for n in [0..3], b such that index ≤ 127.
        // CSD version at CSD[127:126] = r[3][23:22].
        let csd_ver = (r[3] >> 22) & 0x3;

        let sectors = if csd_ver == 1 {
            // CSD v2 (SDHC/SDXC): C_SIZE at CSD[69:48] = r[1][29:8]
            let c_size = ((r[1] >> 8) & 0x3F_FFFF) as u64;
            (c_size + 1) * 1024
        } else {
            // CSD v1 (SDSC):
            // READ_BL_LEN at CSD[83:80] = r[2][11:8]
            let read_bl_len = (r[2] >> 8) & 0xF;
            // C_SIZE at CSD[73:62]: upper 2 bits = r[2][1:0], lower 10 bits = r[1][31:22]
            let c_size = (((r[2] & 0x3) as u64) << 10)
                | (((r[1] >> 22) & 0x3FF) as u64);
            // C_SIZE_MULT at CSD[49:47] = r[1][9:7]
            let c_size_mult = (r[1] >> 7) & 0x7;
            let mult = 1u64 << (c_size_mult + 2);
            let blocknr = (c_size + 1) * mult;
            let blk_len = 1u64 << read_bl_len;
            blocknr * blk_len / 512
        };
        Ok(sectors)
    }
}
