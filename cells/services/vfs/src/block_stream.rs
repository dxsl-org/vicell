//! fatfs block stream backed by the kernel block-I/O syscalls (ids 500/501).
//!
//! Exposes the VirtIO block device to the `fatfs` crate as a seekable byte
//! stream. All I/O is sector-granular: reads/writes do read-modify-write on
//! 512-byte sectors. No unsafe code: all I/O goes through the ostd wrappers.

use ostd::syscall::{sys_blk_flush, sys_blk_read, sys_blk_write};

const SECTOR_SIZE: u64 = 512;

pub struct BlockStream {
    /// Byte position within the FAT16 volume (LBA 0 = byte 0).
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

// Static sector scratch buffers for block I/O.
//
// VFS is a single-threaded cooperative cell (no preemption, no concurrent
// BlockStream I/O).  A static buffer is therefore safe and — crucially —
// avoids placing 512 bytes on the VFS call stack on every fatfs sector
// access.  Deep operations such as recursive directory removal nest many
// fatfs I/O calls; each stack-allocated `[u8; 512]` pushes the stack closer
// to its limit and eventually causes a store page fault.
//
// DMA correctness: `VirtioHal::share()` now performs a kernel page-table walk
// (Phase X-1) to obtain the true physical address, so non-identity-mapped
// static buffers are safe DMA targets.
static mut RD_SEC: [u8; 512] = [0u8; 512];
static mut WR_SEC: [u8; 512] = [0u8; 512];

impl fatfs::Read for BlockStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        if buf.is_empty() {
            return Ok(0);
        }
        let sector = self.pos / SECTOR_SIZE;
        let off    = (self.pos % SECTOR_SIZE) as usize;
        // SAFETY: VFS runs as a single cooperative task; RD_SEC is never
        // accessed re-entrantly (no preemption, no concurrent I/O).
        let sec = unsafe { &mut RD_SEC };
        if !sys_blk_read(sector, sec) {
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

            // SAFETY: VFS is single-threaded; WR_SEC is never accessed re-entrantly.
            let sec = unsafe { &mut WR_SEC };

            if off == 0 && chunk == SECTOR_SIZE as usize {
                sec.copy_from_slice(&buf[written..written + 512]);
                if !sys_blk_write(sector, sec) {
                    return Err(());
                }
            } else {
                // Partial sector — read-modify-write.
                if !sys_blk_read(sector, sec) {
                    return Err(());
                }
                sec[off..off + chunk].copy_from_slice(&buf[written..written + chunk]);
                if !sys_blk_write(sector, sec) {
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
