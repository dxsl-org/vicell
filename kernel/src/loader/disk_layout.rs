//! Constants shared between the kernel early loader and the disk-image builder.
//!
//! The cell bootstrap section is appended AFTER the primary FAT32 partition in
//! `disk_v3.img`.  The FAT32 filesystem starts at LBA 0 and occupies the first
//! `CELL_TABLE_BASE_LBA` sectors.  The kernel reads the cell table from
//! `CELL_TABLE_BASE_LBA` onwards using the VirtIO block driver directly,
//! before any userspace VFS Cell is running.
//!
//! Layout of the cell bootstrap section:
//!
//! ```text
//! LBA CELL_TABLE_BASE_LBA + 0   : CellTableHeader  (one sector = 512 bytes)
//! LBA CELL_TABLE_BASE_LBA + 1   : CellEntry[0..MAX_CELL_ENTRIES]
//!                                   (one sector per entry, padded to 512 bytes)
//! LBA CELL_TABLE_BASE_LBA + 1 + MAX_CELL_ENTRIES : raw ELF data, concatenated
//!                                   (each ELF starts at its entry's `data_lba`)
//! ```

/// Sector offset (from LBA 0) where the cell bootstrap section begins.
/// FAT32 data volume occupies LBA 0-525823 (~257 MB, 65595+ data clusters).
/// 512 sectors of padding follow (525824-526335) before the table at 526336.
pub const CELL_TABLE_BASE_LBA: u64 = 526_336;

/// Magic bytes at the start of `CellTableHeader`; identifies a valid table.
pub const CELL_TABLE_MAGIC: u64 = 0x5649_4F53_5F43_454C; // "ViCell_CEL" in ASCII

/// Maximum number of cells that can appear in the bootstrap table.
pub const MAX_CELL_ENTRIES: usize = 32;

/// Maximum path length (bytes) for a cell path in the bootstrap table.
pub const CELL_PATH_LEN: usize = 64;

/// Maximum path length accepted by the `SpawnFromPath` syscall.
/// Must be ≥ `CELL_PATH_LEN`; defines the trust-boundary validation limit.
pub const MAX_CELL_PATH: usize = 256;

/// Size of one disk sector in bytes.
pub const SECTOR_SIZE: usize = 512;

/// Header at `CELL_TABLE_BASE_LBA + 0`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CellTableHeader {
    /// Must equal `CELL_TABLE_MAGIC`; reject the table otherwise.
    pub magic: u64,
    /// Number of valid entries in the entry array that follows.
    pub count: u32,
    /// Reserved / zero-padded to fill the sector.
    pub _pad: [u8; 500],
}

/// One entry in the cell table; stored starting at `CELL_TABLE_BASE_LBA + 1`.
/// Each entry is padded to exactly `SECTOR_SIZE` bytes.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CellEntry {
    /// Null-terminated path (e.g. `/bin/vfs\0`).
    pub path: [u8; CELL_PATH_LEN],
    /// First LBA of the ELF data.
    pub data_lba: u64,
    /// Size of the ELF data in bytes (not rounded to sectors).
    pub data_size: u64,
    /// Reserved. (512 − 64 − 8 − 8 = 432 bytes)
    pub _pad: [u8; 432],
}

// Compile-time size checks: each header/entry must fit in one sector.
const _: () = assert!(core::mem::size_of::<CellTableHeader>() == SECTOR_SIZE);
const _: () = assert!(core::mem::size_of::<CellEntry>() == SECTOR_SIZE);
