//! FAT32 Filesystem Implementation
use alloc::boxed::Box;
use alloc::string::String;
// use alloc::string::{String, ToString};
use alloc::sync::Arc;
// use alloc::vec::Vec;
// use core::cell::RefCell;
use core::cmp;

use crate::sync::Spinlock;
use crate::task::drivers::ramdisk::ViRamDisk; // Use RAM disk instead of VirtIO
use api::block::ViBlockDevice;
use api::fs::{BoxFuture, FileResult, OpenMode, ViFileSystem, ViFile};
use types::{ViError, ViResult}; // Using Spinlock for kernel level sync

// Import io traits from fatfs (0.4)
use fatfs::{IoBase, Read, Seek, SeekFrom, Write};

/// Wrapper around the Block Device to provide a Read/Write/Seek stream for fatfs
pub struct BlockStream {
    device: ViRamDisk,
    pos: u64,
}

impl BlockStream {
    pub fn new() -> Self {
        Self {
            device: ViRamDisk,
            pos: 0,
        }
    }
}

// Implement fatfs IO traits
// fatfs 0.4 Read/Write/Seek traits inherit from IoBase

impl IoBase for BlockStream {
    type Error = (); // Use unit type for now to satisfy IoBase
}

impl Read for BlockStream {
    // type Error is in IoBase

    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if buf.len() == 0 {
            return Ok(0);
        }

        let sector_size = self.device.sector_size() as u64;
        let start_sector = self.pos / sector_size;
        let offset = (self.pos % sector_size) as usize;

        // Disable log to avoid recursion if necessary, but we need it now.
        // log::info!("BlockStream::read: Pos{}, Sec{}, Off{}, Len{}", self.pos, start_sector, offset, buf.len());

        let mut sector_buf = [0u8; 512];
        if self
            .device
            .read_sector(start_sector, &mut sector_buf)
            .is_err()
        {
            log::error!("BlockStream: Read Error at Sector {}", start_sector);
            return Err(());
        }

        let available = 512 - offset;
        let to_copy = cmp::min(available, buf.len());

        buf[0..to_copy].copy_from_slice(&sector_buf[offset..offset + to_copy]);

        self.pos += to_copy as u64;
        Ok(to_copy)
    }
}

impl Write for BlockStream {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let sector_size = self.device.sector_size() as u64;
        let mut buf_offset = 0;

        while buf_offset < buf.len() {
            let start_sector = self.pos / sector_size;
            let offset = (self.pos % sector_size) as usize;
            
            // Optimization: If writing full sector, skip read-modify-write
            let bytes_left = buf.len() - buf_offset;
            let current_chunk = cmp::min(bytes_left, (sector_size as usize) - offset);

            if offset == 0 && current_chunk == (sector_size as usize) {
                 // Full sector write
                 if self.device.write_sector(start_sector, &buf[buf_offset..buf_offset + current_chunk]).is_err() {
                     return Err(());
                 }
            } else {
                 // Read-Modify-Write
                 let mut sector_buf = [0u8; 512];
                 if self.device.read_sector(start_sector, &mut sector_buf).is_err() {
                     return Err(());
                 }
                 
                 sector_buf[offset..offset + current_chunk].copy_from_slice(&buf[buf_offset..buf_offset + current_chunk]);
                 
                 if self.device.write_sector(start_sector, &sector_buf).is_err() {
                     return Err(());
                 }
            }
            
            buf_offset += current_chunk;
            self.pos += current_chunk as u64;
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl Seek for BlockStream {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        let new_pos = match pos {
            SeekFrom::Start(off) => off,
            SeekFrom::Current(off) => (self.pos as i64 + off) as u64,
            SeekFrom::End(_) => return Err(()),
        };
        self.pos = new_pos;
        Ok(new_pos)
    }
}

// Type alias for the FS
type FatFS = fatfs::FileSystem<BlockStream, fatfs::NullTimeProvider, fatfs::LossyOemCpConverter>;

pub struct ViFatFS {
    // Shared access to the filesystem via Spinlock to ensure Sync
    inner: Arc<Spinlock<FatFS>>,
}

unsafe impl Send for ViFatFS {}
unsafe impl Sync for ViFatFS {}

impl ViFatFS {
    pub fn new() -> ViResult<Self> {
        let mut stream = BlockStream::new();

        // Debug: Read and log boot sector
        {
            let mut boot_sector = [0u8; 512];
            if stream.read(&mut boot_sector).is_ok() {
                log::info!("Boot Sector Debug:");
                log::info!(
                    "  Signature: 0x{:02X}{:02X}",
                    boot_sector[511],
                    boot_sector[510]
                );
                log::info!(
                    "  Bytes/Sector: {}",
                    u16::from_le_bytes([boot_sector[11], boot_sector[12]])
                );
                log::info!("  Sectors/Cluster: {}", boot_sector[13]);
                log::info!(
                    "  Reserved Sectors: {}",
                    u16::from_le_bytes([boot_sector[14], boot_sector[15]])
                );
                log::info!("  FAT Count: {}", boot_sector[16]);
                log::info!("  Media: 0x{:02X}", boot_sector[21]);
                log::info!(
                    "  Total Sectors: {}",
                    u32::from_le_bytes([
                        boot_sector[32],
                        boot_sector[33],
                        boot_sector[34],
                        boot_sector[35]
                    ])
                );
                // FAT16 stores the FAT size in the 16-bit BPB_FATSz16 (offset 22);
                // BPB_FATSz32 (offset 36) is zero on FAT16. Read the FAT16 field.
                log::info!(
                    "  Sectors/FAT (FAT16): {}",
                    u16::from_le_bytes([boot_sector[22], boot_sector[23]])
                );
                log::info!(
                    "  Root Entry Count: {}",
                    u16::from_le_bytes([boot_sector[17], boot_sector[18]])
                );
            }
            // Reset stream position
            stream.pos = 0;
        }

        let options = fatfs::FsOptions::new().update_accessed_date(false);
        match fatfs::FileSystem::new(stream, options) {
            Ok(fs) => Ok(Self {
                inner: Arc::new(Spinlock::new(fs)),
            }),
            Err(e) => {
                log::error!("ViFatFS: Mount failed: {:?}", e);
                Err(ViError::InvalidArgument)
            }
        }
    }
}

impl ViFileSystem for ViFatFS {
    fn open(&self, path: &str, mode: OpenMode) -> ViResult<Box<dyn ViFile + Send + Sync>> {
        // Trim leading slash for fatfs
        let rel_path = path.trim_start_matches('/');
        let fs_lock = self.inner.lock();
        let root = fs_lock.root_dir();
        
        // Determine intent
        let can_create = match mode {
             OpenMode::Write | OpenMode::ReadWrite => true,
             _ => false,
        };

        let mut is_dir = false;

        // Try opening as file first
        if rel_path.is_empty() {
             // Root directory
             is_dir = true;
        } else if root.open_file(rel_path).is_err() {
            // Try opening as directory
            if root.open_dir(rel_path).is_ok() {
                is_dir = true;
            } else {
                // Not found. Create if allowed?
                if can_create && !rel_path.is_empty() {
                     // Try create file
                     if root.create_file(rel_path).is_ok() {
                         // Created successfully
                     } else {
                         return Err(ViError::NotFound);
                     }
                } else {
                     return Err(ViError::NotFound);
                }
            }
        }
        
        // If we just created it, open it again? 
        // create_file returns File object, but we dropped it.
        // Re-open in FatFile is handled lazily in read/write?
        // No, current `FatFile` re-opens on every read/write call?!
        // CHECK FatFile implementation:
        // read() calls `root.open_file(rel_path)`.
        // This is inefficient (Stateless), but works.
        // So successful creation here is enough.
        
        Ok(Box::new(FatFile {
            path: String::from(path),
            pos: 0,
            fs: self.inner.clone(),
            is_dir,
        }))
    }

    fn mkdir(&self, path: &str) -> ViResult<()> {
        let rel_path = path.trim_start_matches('/');
        let fs_lock = self.inner.lock();
        let root = fs_lock.root_dir();
        if root.create_dir(rel_path).is_ok() {
            Ok(())
        } else {
            Err(ViError::IO)
        }
    }

    fn remove(&self, path: &str) -> ViResult<()> {
        let rel_path = path.trim_start_matches('/');
        let fs_lock = self.inner.lock();
        let root = fs_lock.root_dir();
        if root.remove(rel_path).is_ok() {
            Ok(())
        } else {
            Err(ViError::IO)
        }
    }
}

/// Stateless File Handle
pub struct FatFile {
    path: String,
    pos: u64,
    fs: Arc<Spinlock<FatFS>>,
    is_dir: bool,
}

impl ViFile for FatFile {
    fn read(&mut self, buf: &mut [u8]) -> ViResult<usize> {
        if self.is_dir {
            return Err(ViError::IsADirectory);
        }

        let n = {
            let fs_lock = self.fs.lock();
            let root = fs_lock.root_dir();
            // Important: Strip leading slash, same as open()
            let rel_path = self.path.trim_start_matches('/');
            // Bind via `?` so the open_file() Result temporary is dropped at this
            // statement — keeping a `match` scrutinee alive would extend the borrow
            // of `fs_lock` past the end of the block (E0597).
            let mut file = root.open_file(rel_path).map_err(|_| ViError::NotFound)?;
            file.seek(SeekFrom::Start(self.pos))
                .map_err(|_| ViError::IO)?;
            // Loop: fatfs Read::read may return fewer bytes than requested
            // (e.g. stopping at a cluster boundary).  Fill the buffer fully.
            let mut total = 0usize;
            while total < buf.len() {
                let r = file.read(&mut buf[total..]).map_err(|_| ViError::IO)?;
                if r == 0 { break; } // EOF
                total += r;
            }
            total
        };
        self.pos += n as u64;
        Ok(n)
    }

    fn write(&mut self, buf: &[u8]) -> ViResult<usize> {
        if self.is_dir {
             return Err(ViError::IsADirectory);
        }
        let n = {
             let fs_lock = self.fs.lock();
             let root = fs_lock.root_dir();
             let rel_path = self.path.trim_start_matches('/');
             // For write, the file must exist or have been created by open()
             let res = match root.open_file(rel_path) {
                 Ok(mut file) => {
                     file.seek(SeekFrom::Start(self.pos)).map_err(|_| ViError::IO)?;
                     file.write(buf).map_err(|_| ViError::IO)?
                 }
                 Err(_) => return Err(ViError::NotFound),
             };
             res
        };
        self.pos += n as u64;
        Ok(n)
    }

    fn seek(&mut self, pos: api::fs::SeekFrom) -> ViResult<u64> {
        let new_pos = match pos {
            api::fs::SeekFrom::Start(off) => off,
            api::fs::SeekFrom::Current(off) => (self.pos as i64 + off) as u64,
            api::fs::SeekFrom::End(off) => {
                // We need file size to seek from end
                let fs_lock = self.fs.lock();
                let root = fs_lock.root_dir();
                let rel_path = self.path.trim_start_matches('/');
                let mut f = root.open_file(rel_path).map_err(|_| ViError::NotFound)?;
                let size = f.seek(SeekFrom::End(0)).map_err(|_| ViError::IO)?;
                (size as i64 + off) as u64
            }
        };
        self.pos = new_pos;
        Ok(new_pos)
    }

    fn is_dir(&self) -> bool {
        self.is_dir
    }

    fn read_dir(&mut self) -> ViResult<Option<types::DirEntry>> {
        if !self.is_dir {
            return Err(ViError::NotADirectory);
        }

        let fs_lock = self.fs.lock();
        let root = fs_lock.root_dir();
        // Open directory relative to root (assuming path is absolute/full)
        // fatfs doesn't support absolute paths directly if they start with /, usually relative to dir.
        // We trim leading / if present.
        let p = self.path.trim_start_matches('/');
        let dir = if p.is_empty() {
            root
        } else {
            root.open_dir(p).map_err(|_| ViError::NotFound)?
        };

        // Skip 'pos' entries
        let mut skipped = 0;
        let iter = dir.iter();
        for entry_res in iter {
            if skipped < self.pos {
                skipped += 1;
                continue;
            }
            // Found our entry
            let entry = entry_res.map_err(|_| ViError::IO)?;
            let mut name = [0u8; 64];
            let name_str = entry.file_name();
            let name_bytes = name_str.as_bytes();
            let copy_len = core::cmp::min(name.len(), name_bytes.len());
            name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);

            let file_type = if entry.is_dir() {
                types::FileType::Directory
            } else {
                types::FileType::File
            };

            self.pos += 1;
            return Ok(Some(types::DirEntry {
                name,
                file_type,
                size: entry.len(),
            }));
        }

        Ok(None) // EOF
    }

    fn size(&mut self) -> ViResult<u64> {
        if self.is_dir {
            return Err(ViError::IsADirectory);
        }
        let fs_lock = self.fs.lock();
        let root = fs_lock.root_dir();
        let rel_path = self.path.trim_start_matches('/');
        // Re-open fresh so self.pos is not modified (FatFile is stateless per-op).
        let mut f = root.open_file(rel_path).map_err(|_| ViError::NotFound)?;
        f.seek(SeekFrom::End(0)).map_err(|_| ViError::IO)
    }

    fn truncate(&mut self, len: u64) -> ViResult<()> {
        if self.is_dir {
            return Err(ViError::IsADirectory);
        }
        // NOTE: FatFile is stateless (re-opens per operation). If multiple FatFile
        // handles point to the same path, each tracks its own `pos` independently.
        // After truncation the caller's `pos` is clamped; other handles are unaffected
        // and may hold a stale cursor past the new EOF — reads on them will return 0.
        let fs_lock = self.fs.lock();
        let root = fs_lock.root_dir();
        let rel_path = self.path.trim_start_matches('/');
        let mut file = root.open_file(rel_path).map_err(|_| ViError::NotFound)?;
        let current_size = file.seek(SeekFrom::End(0)).map_err(|_| ViError::IO)?;
        if len > current_size {
            return Err(ViError::InvalidArgument);
        }
        file.seek(SeekFrom::Start(len)).map_err(|_| ViError::IO)?;
        file.truncate().map_err(|_| ViError::IO)?;
        if self.pos > len {
            self.pos = len;
        }
        Ok(())
    }

    fn read_async(
        self: Box<Self>,
        buf_ptr: usize,
        buf_len: usize,
    ) -> BoxFuture<'static, FileResult<usize>> {
        Box::pin(async move {
            let mut this = self;
            // Create a temporary slice from the user pointer.
            // SAFETY: The kernel guarantees the pointer is valid (mapped to user space)
            // and we rely on the caller to ensure it doesn't race.
            // In a real async driver, we would need to pin user memory.
            let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len) };

            // Perform synchronous read (for now, until RamDisk is async)
            // This still satisfies the architectural requirement of returning a future.
            let res = this.read(buf);

            // Return ownership and result
            // Cast FatFile back to Trait Object
            let trait_obj: Box<dyn ViFile + Send + Sync> = this;
            (trait_obj, res)
        })
    }
}
