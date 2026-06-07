use super::core::{CardInfo, MmcCore};
use super::regs::*;
use hal_traits_mmc::{CardType, ViMmcHost};
use types::{ViError, ViResult};

/// eMMC block device. State is owned here; caller must hold a `Spinlock` guard
/// (via `MMC_DEVICE`) before calling `read_sector` / `write_sector`.
pub struct EmmcBlock {
    pub(super) core: MmcCore,
    pub(super) info: CardInfo,
}

impl EmmcBlock {
    /// Probe the SDHCI controller at `sdhci_base` for an eMMC card.
    ///
    /// Returns `Err(NotFound)` if the controller is absent or the attached card
    /// is not eMMC.
    ///
    /// # Safety
    /// `sdhci_base` must be a valid kernel-mapped MMIO address for the SDHCI register block.
    pub unsafe fn probe(sdhci_base: usize) -> ViResult<Self> {
        // SAFETY: forwarded from caller contract.
        let mut core = MmcCore::new(sdhci_base);
        let info = core.init_card()?;
        if info.card_type != CardType::Emmc {
            return Err(ViError::NotFound);
        }
        log::info!("[emmc] eMMC probed: {} sectors (~{} MiB)",
            info.sector_count, info.sector_count / 2048);
        Ok(Self { core, info })
    }

    pub fn read_sector(&mut self, sector: u64, buf: &mut [u8]) -> ViResult<()> {
        self.core.host.setup_data_transfer(0x0200, 1, TM_DATA_READ);
        let cmd = hal_traits_mmc::MmcCmd {
            index: 17,
            arg: sector as u32,
            resp_type: hal_traits_mmc::RespType::R1,
            has_data: true,
        };
        self.core.host.send_cmd(cmd)?;
        self.core.host.read_block(buf)
    }

    pub fn write_sector(&mut self, sector: u64, buf: &[u8]) -> ViResult<()> {
        self.core.host.setup_data_transfer(0x0200, 1, 0x0000);
        let cmd = hal_traits_mmc::MmcCmd {
            index: 24,
            arg: sector as u32,
            resp_type: hal_traits_mmc::RespType::R1,
            has_data: true,
        };
        self.core.host.send_cmd(cmd)?;
        self.core.host.write_block(buf)
    }

    pub fn sector_count(&self) -> u64 { self.info.sector_count }
}

impl Drop for EmmcBlock {
    fn drop(&mut self) {
        // SdhciController::drop powers off the card slot on teardown.
    }
}
