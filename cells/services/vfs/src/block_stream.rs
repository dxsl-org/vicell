//! fatfs block stream backed by the kernel block-I/O syscalls (ids 500/501).
//!
//! Exposes the VirtIO block device to the `fatfs` crate as a seekable byte
//! stream. All I/O is sector-granular: reads/writes do read-modify-write on
//! 512-byte sectors. No unsafe code: all I/O goes through the ostd wrappers.

use ostd::syscall::{sys_blk_read, sys_blk_write};

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

impl fatfs::Read for BlockStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        if buf.is_empty() {
            return Ok(0);
        }
        let sector = self.pos / SECTOR_SIZE;
        let off    = (self.pos % SECTOR_SIZE) as usize;
        let mut sec = [0u8; 512];
        if !sys_blk_read(sector, &mut sec) {
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
                let mut full = [0u8; 512];
                full.copy_from_slice(&buf[written..written + 512]);
                if !sys_blk_write(sector, &full) {
                    return Err(());
                }
            } else {
                // Partial sector — read-modify-write.
                let mut sec = [0u8; 512];
                if !sys_blk_read(sector, &mut sec) {
                    return Err(());
                }
                sec[off..off + chunk].copy_from_slice(&buf[written..written + chunk]);
                if !sys_blk_write(sector, &sec) {
                    return Err(());
                }
            }

            written    += chunk;
            self.pos   += chunk as u64;
        }
        Ok(written)
    }

    /// VirtIO writes are synchronous (polling) — durable on return; no-op flush.
    fn flush(&mut self) -> Result<(), ()> {
        Ok(())
    }
}

impl fatfs::Seek for BlockStream {
    fn seek(&mut self, pos: fatfs::SeekFrom) -> Result<u64, ()> {
        self.pos = match pos {
            fatfs::SeekFrom::Start(n)   => n,
            fatfs::SeekFrom::Current(n) => (self.pos as i64 + n) as u64,
            // SeekFrom::End is not used by the Phase D write path (remove+create
            // semantics avoid append/truncate). Return Err if fatfs calls it.
            fatfs::SeekFrom::End(_)     => return Err(()),
        };
        Ok(self.pos)
    }
}
