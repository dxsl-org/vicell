//! littlefs storage driver over the VirtIO block device — MBR partition P4.
//!
//! Byte offsets from littlefs are translated to absolute LBAs inside P4
//! (`api::disk::PART_LFS_BASE_LBA`); the kernel's `check_block_access` gate
//! independently confines this cell to P1+P4, so even a littlefs bug cannot
//! reach the cell table or snapshot regions.
//!
//! POWER-SAFETY INVARIANT: every read/write goes straight to the block
//! syscalls — littlefs's copy-on-write correctness depends on its prog/erase
//! ordering reaching the device. Do NOT route this through the FAT PageCache.

use littlefs2::consts::{U16, U512};
use littlefs2::driver::Storage;
use littlefs2::io::{Error, Result};
use ostd::syscall::{sys_blk_read, sys_blk_write};

const SECTOR: usize = 512;

pub struct LfsDisk;

impl Storage for LfsDisk {
    const READ_SIZE: usize = SECTOR;
    const WRITE_SIZE: usize = SECTOR;
    /// Erase block = 4 KiB (8 sectors): small enough for low write
    /// amplification on a virtual disk, large enough to keep metadata
    /// pair churn reasonable.
    const BLOCK_SIZE: usize = 4096;
    const BLOCK_COUNT: usize = (api::disk::PART_LFS_SECTORS as usize) * SECTOR / 4096;

    type CACHE_SIZE = U512;
    type LOOKAHEAD_SIZE = U16;

    fn read(&mut self, off: usize, buf: &mut [u8]) -> Result<usize> {
        debug_assert!(off % SECTOR == 0 && buf.len() % SECTOR == 0);
        let base = api::disk::PART_LFS_BASE_LBA + (off / SECTOR) as u64;
        let mut sec = [0u8; SECTOR];
        for (i, chunk) in buf.chunks_mut(SECTOR).enumerate() {
            if !sys_blk_read(base + i as u64, &mut sec) {
                return Err(Error::IO);
            }
            chunk.copy_from_slice(&sec);
        }
        Ok(buf.len())
    }

    fn write(&mut self, off: usize, data: &[u8]) -> Result<usize> {
        debug_assert!(off % SECTOR == 0 && data.len() % SECTOR == 0);
        let base = api::disk::PART_LFS_BASE_LBA + (off / SECTOR) as u64;
        let mut sec = [0u8; SECTOR];
        for (i, chunk) in data.chunks(SECTOR).enumerate() {
            sec.copy_from_slice(chunk);
            if !sys_blk_write(base + i as u64, &sec) {
                return Err(Error::IO);
            }
        }
        Ok(data.len())
    }

    /// VirtIO block storage overwrites in place — no erase cycle exists, so
    /// this is a no-op. littlefs only requires that a block it "erased" can
    /// subsequently be programmed, which plain overwrite satisfies.
    fn erase(&mut self, _off: usize, len: usize) -> Result<usize> {
        Ok(len)
    }
}
