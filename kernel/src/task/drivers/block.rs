use api::block::ViBlockDevice;
use super::blk_nvme::NvmeBlk;
use super::mmc::MmcBlock;
use super::virtio_blk::viVirtIOBlk;
use types::ViResult;

static NVME_ZST:   NvmeBlk     = NvmeBlk;
static VIRTIO_ZST: viVirtIOBlk = viVirtIOBlk;
static MMC_ZST:    MmcBlock    = MmcBlock;

/// Return the active block device.
///
/// Priority: NVMe (PCIe, highest — real G2 storage) → VirtIO (QEMU) → MMC (real board).
/// Falls back to the VirtIO ZST (returns `Err(NotFound)` gracefully) when nothing probed.
///
/// NVMe is checked first so that an x86_64 q35 guest with `-device nvme` boots from
/// the NVMe disk rather than a secondary VirtIO disk. Both can coexist; first match wins.
pub fn block_device() -> &'static dyn ViBlockDevice {
    if super::blk_nvme::is_present() {
        &NVME_ZST
    } else if super::virtio_blk::is_present() {
        &VIRTIO_ZST
    } else if super::mmc::is_present() {
        &MMC_ZST
    } else {
        &VIRTIO_ZST
    }
}

/// Read one 512-byte sector. Convenience wrapper — no `ViBlockDevice` import required.
pub fn read_sector(sector: u64, buf: &mut [u8]) -> ViResult<()> {
    block_device().read_sector(sector, buf)
}

/// Write one 512-byte sector. Convenience wrapper — no `ViBlockDevice` import required.
pub fn write_sector(sector: u64, buf: &[u8]) -> ViResult<()> {
    block_device().write_sector(sector, buf)
}

/// Flush pending writes. Convenience wrapper — no `ViBlockDevice` import required.
pub fn flush() -> ViResult<()> {
    block_device().flush()
}
