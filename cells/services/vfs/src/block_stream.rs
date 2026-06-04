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

impl fatfs::Read for BlockStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        if buf.is_empty() {
            return Ok(0);
        }
        let sector = self.pos / SECTOR_SIZE;
        let off    = (self.pos % SECTOR_SIZE) as usize;
        // Stack-allocated sector buffer.  VirtIO DMA requires identity-mapped
        // buffers; stack pages ARE identity-mapped in ViOS SAS.  Phase X-1
        // increased STACK_PAGES to 64 (256 KB) so the deep fatfs nesting
        // during recursive directory removal no longer overflows the stack.
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
                let mut full = [0u8; 512]; // stack-allocated (VirtIO DMA constraint)
                full.copy_from_slice(&buf[written..written + 512]);
                if !sys_blk_write(sector, &full) {
                    return Err(());
                }
            } else {
                // Partial sector — read-modify-write.
                let mut sec = [0u8; 512]; // stack-allocated (VirtIO DMA constraint)
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
