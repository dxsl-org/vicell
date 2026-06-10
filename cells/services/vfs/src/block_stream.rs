//! fatfs block stream backed by the kernel block-I/O syscalls (ids 500/501).
//!
//! Two types are exported:
//!  - `BlockStream`: raw sector I/O, no caching. Used internally by `PageCache`.
//!  - `CachedBlockStream`: wraps `BlockStream + PageCache`; implement all fatfs
//!    traits. VFS mounts FAT32 through this type.

use crate::page_cache::PageCache;
use ostd::syscall::{sys_blk_flush, sys_blk_read, sys_blk_write};

const SECTOR_SIZE: u64 = 512;

/// First absolute LBA of the FAT32 volume — MBR partition P1 (Milestone 2.5
/// Phase 03; see `tools/write-mbr.py` and `kernel/src/loader/disk_layout.rs`).
/// `pos` stays partition-relative; only the syscall LBA gets the offset, so
/// fatfs and the page cache never see absolute sector numbers.
/// TODO(P03 step B): replace with the shared `api::disk` constant once the
/// Law-1 change lands.
const FAT_PART_BASE_LBA: u64 = 2_048;

pub struct BlockStream {
    /// Byte position within the FAT32 volume (LBA 0 = byte 0).
    pos: u64,
}

impl BlockStream {
    pub fn new() -> Self {
        Self { pos: 0 }
    }
}

impl fatfs::IoBase for BlockStream {
    type Error = ();
}

impl fatfs::Read for BlockStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        if buf.is_empty() {
            return Ok(0);
        }
        let sector = self.pos / SECTOR_SIZE;
        let off    = (self.pos % SECTOR_SIZE) as usize;
        // Stack-allocated sector buffer.  VirtIO DMA requires identity-mapped
        // buffers; stack pages ARE identity-mapped in ViCell SAS.  Phase X-1
        // increased STACK_PAGES to 64 (256 KB) so the deep fatfs nesting
        // during recursive directory removal no longer overflows the stack.
        let mut sec = [0u8; 512];
        if !sys_blk_read(FAT_PART_BASE_LBA + sector, &mut sec) {
            return Err(());
        }
        let n = core::cmp::min(SECTOR_SIZE as usize - off, buf.len());
        buf[..n].copy_from_slice(&sec[off..off + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl fatfs::Write for BlockStream {
    fn write(&mut self, buf: &[u8]) -> Result<usize, ()> {
        let mut written = 0usize;
        while written < buf.len() {
            let sector = self.pos / SECTOR_SIZE;
            let off    = (self.pos % SECTOR_SIZE) as usize;
            let chunk  = core::cmp::min(buf.len() - written, SECTOR_SIZE as usize - off);

            if off == 0 && chunk == SECTOR_SIZE as usize {
                // Full-sector write — no need to read first.
                let mut full = [0u8; 512]; // stack-allocated (VirtIO DMA constraint)
                full.copy_from_slice(&buf[written..written + 512]);
                if !sys_blk_write(FAT_PART_BASE_LBA + sector, &full) {
                    return Err(());
                }
            } else {
                // Partial sector — read-modify-write.
                let mut sec = [0u8; 512]; // stack-allocated (VirtIO DMA constraint)
                if !sys_blk_read(FAT_PART_BASE_LBA + sector, &mut sec) {
                    return Err(());
                }
                sec[off..off + chunk].copy_from_slice(&buf[written..written + chunk]);
                if !sys_blk_write(FAT_PART_BASE_LBA + sector, &sec) {
                    return Err(());
                }
            }

            written    += chunk;
            self.pos   += chunk as u64;
        }
        Ok(written)
    }

    /// Issue a VirtIO FLUSH command so prior writes reach the backing disk image.
    ///
    /// Required for reboot persistence: fatfs calls `flush()` when a `File` is
    /// dropped, which is the signal to commit metadata to durable storage. Without
    /// a real flush here, a SBI SRST shutdown may discard writes still in QEMU's
    /// write-back buffer before they reach `disk_v3.img`.
    fn flush(&mut self) -> Result<(), ()> {
        if sys_blk_flush() { Ok(()) } else { Err(()) }
    }
}

impl fatfs::Seek for BlockStream {
    fn seek(&mut self, pos: fatfs::SeekFrom) -> Result<u64, ()> {
        self.pos = match pos {
            fatfs::SeekFrom::Start(n)   => n,
            fatfs::SeekFrom::Current(n) => {
                let result = self.pos as i64 + n;
                if result < 0 { return Err(()); }
                result as u64
            }
            fatfs::SeekFrom::End(_)     => return Err(()),
        };
        Ok(self.pos)
    }
}

impl BlockStream {
    /// Read one 512-byte sector directly from disk, bypassing the page cache.
    /// Called by `PageCache` on a cache miss. `sector` is partition-relative.
    pub fn read_raw_sector(&mut self, sector: u64, buf: &mut [u8; 512]) -> bool {
        sys_blk_read(FAT_PART_BASE_LBA + sector, buf)
    }

    /// Write one 512-byte sector directly to disk, bypassing the page cache.
    /// Called by `PageCache::flush_dirty`. `sector` is partition-relative.
    pub fn write_raw_sector(&mut self, sector: u64, data: &[u8; 512]) -> bool {
        sys_blk_write(FAT_PART_BASE_LBA + sector, data)
    }
}

// ── CachedBlockStream ─────────────────────────────────────────────────────────

/// fatfs block stream with an LRU sector cache.
///
/// Replaces `BlockStream` as the fatfs I/O backend. All sector reads are
/// served from `PageCache` on hit; misses fall through to `BlockStream`.
/// Writes use write-through policy (flush on every write) while backed by FAT32.
pub struct CachedBlockStream {
    inner: BlockStream,
    cache: PageCache,
}

impl CachedBlockStream {
    pub fn new() -> Self {
        Self {
            inner: BlockStream::new(),
            cache: PageCache::new(),
        }
    }
}

impl fatfs::IoBase for CachedBlockStream {
    type Error = ();
}

impl fatfs::Read for CachedBlockStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        if buf.is_empty() {
            return Ok(0);
        }
        let sector = self.inner.pos / SECTOR_SIZE;
        let off    = (self.inner.pos % SECTOR_SIZE) as usize;
        let mut sec = [0u8; 512];
        if !self.cache.read_sector(&mut self.inner, sector, &mut sec) {
            return Err(());
        }
        let n = core::cmp::min(SECTOR_SIZE as usize - off, buf.len());
        buf[..n].copy_from_slice(&sec[off..off + n]);
        self.inner.pos += n as u64;
        Ok(n)
    }
}

impl fatfs::Write for CachedBlockStream {
    fn write(&mut self, buf: &[u8]) -> Result<usize, ()> {
        let mut written = 0usize;
        while written < buf.len() {
            let sector = self.inner.pos / SECTOR_SIZE;
            let off    = (self.inner.pos % SECTOR_SIZE) as usize;
            let chunk  = core::cmp::min(buf.len() - written, SECTOR_SIZE as usize - off);

            if off == 0 && chunk == SECTOR_SIZE as usize {
                // Full-sector write: no read-before-write needed.
                let mut full = [0u8; 512];
                full.copy_from_slice(&buf[written..written + 512]);
                if !self.cache.write_sector(&mut self.inner, sector, &full) {
                    return Err(());
                }
            } else {
                // Partial sector: read-modify-write through cache.
                let mut sec = [0u8; 512];
                if !self.cache.read_sector(&mut self.inner, sector, &mut sec) {
                    return Err(());
                }
                sec[off..off + chunk].copy_from_slice(&buf[written..written + chunk]);
                if !self.cache.write_sector(&mut self.inner, sector, &sec) {
                    return Err(());
                }
            }

            written        += chunk;
            self.inner.pos += chunk as u64;
        }
        Ok(written)
    }

    /// Issue a VirtIO FLUSH after write-through cache has already synced sectors.
    ///
    /// fatfs calls `flush()` when a `File` is dropped; this ensures any queued
    /// VirtIO writes reach `disk_v3.img` before QEMU shuts down.
    fn flush(&mut self) -> Result<(), ()> {
        if sys_blk_flush() { Ok(()) } else { Err(()) }
    }
}

impl fatfs::Seek for CachedBlockStream {
    fn seek(&mut self, pos: fatfs::SeekFrom) -> Result<u64, ()> {
        // Delegate entirely to the inner stream's position tracking.
        self.inner.seek(pos)
    }
}
