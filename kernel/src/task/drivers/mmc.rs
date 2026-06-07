pub mod core;
pub mod emmc;
pub mod regs;
pub mod sd;
pub mod sdhci;

use crate::sync::Spinlock;
use api::block::ViBlockDevice;
use emmc::EmmcBlock;
use sd::SdBlock;
use types::{ViError, ViResult};

// ---------------------------------------------------------------------------
// Compile-time board SDHCI base address selection.
//
// Build with `--features board-rpi4` or `--features board-visionfive2`.
// Without either feature the driver is compiled but init_driver() is a no-op,
// so QEMU boots continue to use VirtIO block unchanged.
// ---------------------------------------------------------------------------

#[cfg(feature = "board-rpi4")]
const SDHCI_BASE: usize = 0xFE34_0000; // BCM2711 Arasan eMMC2

#[cfg(feature = "board-vf2")]
const SDHCI_BASE: usize = 0x1604_0000; // JH7110 SDHCI (SDIO1)

#[cfg(not(any(feature = "board-rpi4", feature = "board-vf2")))]
const SDHCI_BASE: usize = 0x0; // no real board configured

// ---------------------------------------------------------------------------
// Device enum — holds either eMMC or SD (runtime probe selection).
// ---------------------------------------------------------------------------

enum MmcDevice {
    Emmc(EmmcBlock),
    Sd(SdBlock),
}

impl MmcDevice {
    fn read_sector(&mut self, sector: u64, buf: &mut [u8]) -> ViResult<()> {
        match self {
            Self::Emmc(d) => d.read_sector(sector, buf),
            Self::Sd(d) => d.read_sector(sector, buf),
        }
    }
    fn write_sector(&mut self, sector: u64, buf: &[u8]) -> ViResult<()> {
        match self {
            Self::Emmc(d) => d.write_sector(sector, buf),
            Self::Sd(d) => d.write_sector(sector, buf),
        }
    }
    fn sector_count(&self) -> u64 {
        match self {
            Self::Emmc(d) => d.sector_count(),
            Self::Sd(d) => d.sector_count(),
        }
    }
}

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

static MMC_DEVICE: Spinlock<Option<MmcDevice>> = Spinlock::new(None);

/// Zero-sized struct; implements [`ViBlockDevice`] by locking [`MMC_DEVICE`].
pub struct MmcBlock;

impl ViBlockDevice for MmcBlock {
    fn read_sector(&self, sector: u64, buf: &mut [u8]) -> ViResult<()> {
        MMC_DEVICE.lock().as_mut().ok_or(ViError::NotFound)?.read_sector(sector, buf)
    }
    fn write_sector(&self, sector: u64, buf: &[u8]) -> ViResult<()> {
        MMC_DEVICE.lock().as_mut().ok_or(ViError::NotFound)?.write_sector(sector, buf)
    }
    fn sector_count(&self) -> u64 {
        MMC_DEVICE.lock().as_ref().map(|d| d.sector_count()).unwrap_or(0)
    }
    fn sector_size(&self) -> usize { 512 }
    fn flush(&self) -> ViResult<()> { Ok(()) }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Probe the SDHCI controller for an eMMC or SD card.
///
/// No-op when no board feature is selected (QEMU keeps using VirtIO).
pub fn init_driver() {
    if SDHCI_BASE == 0 {
        log::debug!("[mmc] no board configured — skipping SDHCI probe");
        return;
    }

    // Try eMMC first, then SD card.
    // SAFETY: SDHCI_BASE is the kernel-mapped MMIO address for the configured board.
    // The MMIO region must be mapped before calling init_driver().
    let emmc = unsafe { EmmcBlock::probe(SDHCI_BASE) };
    match emmc {
        Ok(dev) => {
            *MMC_DEVICE.lock() = Some(MmcDevice::Emmc(dev));
            log::info!("[mmc] eMMC probed at 0x{:x}", SDHCI_BASE);
            return;
        }
        Err(e) => log::debug!("[mmc] eMMC probe failed ({:?}), trying SD...", e),
    }

    let sd = unsafe { SdBlock::probe(SDHCI_BASE) };
    match sd {
        Ok(dev) => {
            *MMC_DEVICE.lock() = Some(MmcDevice::Sd(dev));
            log::info!("[mmc] SD card probed at 0x{:x}", SDHCI_BASE);
        }
        Err(e) => log::warn!("[mmc] no card found at 0x{:x}: {:?}", SDHCI_BASE, e),
    }
}

/// Returns `true` when an MMC/SD card was successfully probed.
pub fn is_present() -> bool {
    MMC_DEVICE.lock().is_some()
}

/// Force-release the `MMC_DEVICE` lock from the fault/panic path.
///
/// # Safety
/// Single-hart, interrupts disabled, called only from the fault/panic handler.
pub unsafe fn force_unlock_locks() {
    MMC_DEVICE.force_unlock();
}
