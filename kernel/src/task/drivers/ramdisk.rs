use api::block::ViBlockDevice;
use types::{ViError, ViResult};

/// RAM Disk - Zero-copy block device with embedded FAT32 image
/// Implements Luật 8: Direct memory access without copying
#[allow(non_camel_case_types)]
// pub struct viRamDisk; // Removed duplicate

// Embed the small kernel-internal FAT32 image.
//
// This image contains only the files the kernel's own filesystem serves:
//   /bin/{init,shell,vfs,config,lua}  — release-built cell ELFs
//   /hostname, /readme               — system metadata
//
// The VirtIO block device (disk_v3.img) is a SEPARATE disk that holds the
// cell bootstrap table used by the early loader for SpawnFromPath.
// Keeping the embedded FS small (~8 MB) prevents the kernel binary
// from bloating to 52 MB when disk_v3.img was embedded here.
//
// To regenerate this image run:
//   scripts\update-embedded.ps1          # builds release cells
//   python tools\mkfat32.py kernel\src\embedded\kernel_fs.img ...
// Path is relative to this source file (kernel/src/task/drivers/ramdisk.rs).
// kernel_fs.img lives in kernel/src/embedded/kernel_fs.img.
static DISK_IMAGE: &[u8] = include_bytes!("../../embedded/kernel_fs.img");

const SECTOR_SIZE: usize = 512;

use alloc::vec;
use alloc::vec::Vec;
use crate::sync::Spinlock;

// Global Mutable Storage for the RAM Disk
// We use a Spinlock to ensure thread/core safety.
// The vector is initialized lazily or at boot.
static RAM_DISK_STORAGE: Spinlock<Option<Vec<u8>>> = Spinlock::new(None);

pub struct ViRamDisk;

impl ViRamDisk {
    // Helper to access storage.
    // NOTE: This locks the storage for the duration of the copy, which is fast for 512 bytes.
    fn with_storage<F, R>(op: F) -> ViResult<R>
    where
        F: FnOnce(&mut Vec<u8>) -> ViResult<R>,
    {
        let mut guard = RAM_DISK_STORAGE.lock();
        if let Some(storage) = guard.as_mut() {
            op(storage)
        } else {
            // Not initialized yet? Or panic?
            log::error!("RAM Disk: Storage not initialized!");
            Err(ViError::IO)
        }
    }
}

impl ViBlockDevice for ViRamDisk {
    fn read_sector(&self, sector: u64, buf: &mut [u8]) -> ViResult<()> {
        Self::with_storage(|storage| {
            let offset = (sector as usize) * SECTOR_SIZE;
            if offset + SECTOR_SIZE > storage.len() {
                return Err(ViError::InvalidArgument);
            }
            buf.copy_from_slice(&storage[offset..offset + SECTOR_SIZE]);
            Ok(())
        })
    }

    fn write_sector(&self, sector: u64, buf: &[u8]) -> ViResult<()> {
        Self::with_storage(|storage| {
            let offset = (sector as usize) * SECTOR_SIZE;
            if offset + SECTOR_SIZE > storage.len() {
                return Err(ViError::InvalidArgument);
            }
            storage[offset..offset + SECTOR_SIZE].copy_from_slice(buf);
            Ok(())
        })
    }

    fn sector_count(&self) -> u64 {
        let guard = RAM_DISK_STORAGE.lock();
        if let Some(s) = guard.as_ref() {
            (s.len() / SECTOR_SIZE) as u64
        } else {
            0
        }
    }

    fn sector_size(&self) -> usize {
        SECTOR_SIZE
    }

    fn flush(&self) -> ViResult<()> {
        Ok(())
    }
}

/// Initialize RAM disk
pub fn init_driver() {
    log::info!("RAM Disk: Initializing Mutable Storage...");
    
    // Allocate 40MB on Heap (Might be heavy!)
    let mut storage = vec![0u8; DISK_IMAGE.len()];
    storage.copy_from_slice(DISK_IMAGE);
    
    {
        let mut guard = RAM_DISK_STORAGE.lock();
        *guard = Some(storage);
    }
    
    log::info!("RAM Disk: Embedded FAT32 image loaded into Heap");
    log::info!(
        "  Size: {} KB ({} sectors)",
        DISK_IMAGE.len() / 1024,
        DISK_IMAGE.len() / SECTOR_SIZE
    );
}
