//! Boot-time cell loader — reads cell ELFs directly from the block device.
//!
//! Used during early boot before the VFS Cell is running.  Reads the cell
//! bootstrap section appended to `disk_v3.img` at `CELL_TABLE_BASE_LBA`.
//!
//! Call sequence:
//! 1. `EarlyLoader::probe()` — reads and validates the cell table header.
//! 2. `EarlyLoader::read_file(path)` — returns an owned `Box<[u8]>` of the ELF.

use super::disk_layout::{
    CellEntry, CellTableHeader, CELL_PATH_LEN, CELL_TABLE_BASE_LBA, CELL_TABLE_MAGIC,
    MAX_CELL_ENTRIES, SECTOR_SIZE,
};
use alloc::boxed::Box;
use alloc::vec::Vec;
use types::{ViError, ViResult};

/// Cached cell table loaded from disk at boot.
///
/// `None` until `EarlyLoader::probe()` is called successfully.
static CELL_TABLE: crate::sync::Spinlock<Option<EarlyTable>> =
    crate::sync::Spinlock::new(None);

struct EarlyTable {
    entries: Vec<CellEntry>,
}

/// Boot-time cell loader backed by the VirtIO block driver.
pub struct EarlyLoader;

impl EarlyLoader {
    /// Read the cell bootstrap table from disk and cache it.
    ///
    /// Must be called after the VirtIO block driver is initialised but before
    /// any `read_file` call.  Idempotent — safe to call more than once.
    ///
    /// # Errors
    /// Returns `ViError::NotFound` if no block device is attached.
    /// Returns `ViError::InvalidInput` if the magic bytes do not match
    /// (disk image was not generated with `gen_disk.ps1`).
    pub fn probe() -> ViResult<()> {
        
        

        // Idempotent: skip if already probed.
        if CELL_TABLE.lock().is_some() {
            return Ok(());
        }

        // ── Read header sector ───────────────────────────────────────────────
        let mut header_buf = [0u8; SECTOR_SIZE];
        crate::task::drivers::block::read_sector(CELL_TABLE_BASE_LBA, &mut header_buf)?;

        // SAFETY: header_buf is SECTOR_SIZE bytes aligned to u8; CellTableHeader
        // is repr(C) and also SECTOR_SIZE bytes.  Transmute is safe here.
        let header: CellTableHeader = unsafe {
            core::mem::transmute(header_buf)
        };

        if header.magic != CELL_TABLE_MAGIC {
            log::warn!(
                "[early] cell table magic mismatch: got 0x{:016X}, want 0x{:016X}",
                header.magic,
                CELL_TABLE_MAGIC
            );
            return Err(ViError::InvalidInput);
        }

        let count = header.count as usize;
        if count > MAX_CELL_ENTRIES {
            log::error!("[early] cell table count {} exceeds MAX_CELL_ENTRIES", count);
            return Err(ViError::InvalidInput);
        }

        // ── Read entry sectors ───────────────────────────────────────────────
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let entry_lba = CELL_TABLE_BASE_LBA + 1 + i as u64;
            let mut entry_buf = [0u8; SECTOR_SIZE];
            crate::task::drivers::block::read_sector(entry_lba, &mut entry_buf)?;
            // SAFETY: entry_buf is SECTOR_SIZE bytes; CellEntry is repr(C) SECTOR_SIZE.
            let entry: CellEntry = unsafe { core::mem::transmute(entry_buf) };
            entries.push(entry);
        }

        log::info!("[early] cell table loaded: {} entries", count);
        for e in &entries {
            let path = core::str::from_utf8(&e.path[..CELL_PATH_LEN])
                .unwrap_or("?")
                .trim_end_matches('\0');
            log::debug!("[early]   {} @ LBA {} ({} bytes)", path, e.data_lba, e.data_size);
        }

        *CELL_TABLE.lock() = Some(EarlyTable { entries });
        Ok(())
    }

    /// Read a cell ELF from the bootstrap table into a heap-allocated buffer.
    ///
    /// Falls back to the kernel's embedded FAT16 filesystem (VIFS1) when the
    /// bootstrap table is absent — this allows ARM64 and diskless boots to spawn
    /// cells whose ELFs live in kernel_fs.img rather than a VirtIO block device.
    ///
    /// `path` must match what `gen_disk.ps1` wrote (e.g. `/bin/vfs`).
    ///
    /// # Errors
    /// Returns `ViError::NotFound` if neither the block table nor VIFS1 has the path.
    pub fn read_file(path: &str) -> ViResult<Box<[u8]>> {
        
        

        // Attempt block-device bootstrap table path first.
        let block_result = (|| -> ViResult<Box<[u8]>> {
            let (data_lba, size) = {
                let guard = CELL_TABLE.lock();
                let table = guard.as_ref().ok_or(ViError::NotFound)?;
                let entry = table.entries.iter().find(|e| {
                    let stored = core::str::from_utf8(&e.path[..CELL_PATH_LEN])
                        .unwrap_or("")
                        .trim_end_matches('\0');
                    stored == path
                }).ok_or(ViError::NotFound)?;
                (entry.data_lba, entry.data_size as usize)
            };
            if size == 0 { return Err(ViError::InvalidInput); }
            let sector_count = (size + SECTOR_SIZE - 1) / SECTOR_SIZE;
            let mut buf = alloc::vec![0u8; sector_count * SECTOR_SIZE];
            for i in 0..sector_count {
                let lba = data_lba + i as u64;
                let offset = i * SECTOR_SIZE;
                crate::task::drivers::block::read_sector(lba, &mut buf[offset..offset + SECTOR_SIZE])?;
            }
            buf.truncate(size);
            Ok(buf.into_boxed_slice())
        })();

        if block_result.is_ok() {
            return block_result;
        }

        // Fallback: read from the embedded FAT16 ramdisk (VIFS1).
        // This path is used when no VirtIO block device is present (e.g. ARM64 QEMU
        // without a separate disk image, or CI diskless boots).
        log::debug!("[early] block table miss for {:?} — trying VIFS1", path);
        crate::fs::read_file_from_vifs1(path)
    }
}
