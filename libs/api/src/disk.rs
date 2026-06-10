//! Canonical MBR partition layout for `disk_v3.img` — the contract shared by
//! the kernel (`loader/disk_layout.rs`), cells (VFS block stream, littlefs
//! backend) and the image builders (`tools/write-mbr.py`, `gen_disk.ps1`,
//! `tools/mkfat32_inplace.py` — the Python tools carry a must-match copy).
//!
//! The on-disk MBR at LBA 0 is the runtime source of truth; the kernel parses
//! and cross-checks it at boot (`disk_layout::verify_mbr`, warn-only). These
//! constants stay authoritative so a blank or legacy image still boots.
//!
//! ```text
//! P1  0x0C  FAT32 interop volume      (VFS /data today, /mnt/sd after P04)
//! P2  0x7F  cell bootstrap table      (kernel early loader ONLY — never granted)
//! P3  0x7D  kernel heap snapshot      (Phase 29 — kernel ONLY, never granted)
//! P4  0x7E  littlefs /data            (Milestone 2.5 Phase 04)
//! ```

/// P1: first absolute LBA of the FAT32 volume (MBR-standard 1 MiB alignment).
pub const PART_FAT32_BASE_LBA: u64 = 2_048;
/// P1: FAT32 volume size in sectors.
pub const PART_FAT32_SECTORS: u64 = 524_288;

/// P2: first LBA of the cell bootstrap table (header + entries + ELF blobs).
pub const PART_CELLTBL_BASE_LBA: u64 = 526_336;
/// P2 size in sectors (16 MiB).
pub const PART_CELLTBL_SECTORS: u64 = 33_664;

/// P3: kernel heap snapshot region (raw LBA, no filesystem).
pub const PART_SNAPSHOT_BASE_LBA: u64 = 560_000;
/// P3 size in sectors (~117 MB — covers the full 64 MB kernel heap + headers).
pub const PART_SNAPSHOT_SECTORS: u64 = 240_000;

/// P4: littlefs persistent /data store.
pub const PART_LFS_BASE_LBA: u64 = 800_000;
/// P4 size in sectors (64 MB).
pub const PART_LFS_SECTORS: u64 = 131_072;
