// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// `no_std` is disabled when the test harness is active so `#[test]` functions
// can link against the host `std`.  All production builds remain bare-metal.
#![cfg_attr(not(test), no_std)]

//! Core types for ViCell Cellular SAS architecture.
//!
//! This crate defines fundamental types used across the entire system.

/// Kernel Result Type
pub type HalResult<T> = core::result::Result<T, HalError>;

/// Standard Result type for ViCell APIs.
pub type Result<T, E = ViError> = core::result::Result<T, E>;

/// Kernel/HAL Errors
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HalError {
    GenericError,
    BusError,
    InvalidDevice,
    NotSupported,
    Busy,
    IoError,
    InvalidInput,
}

/// Unique identifier for a Cell.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CellId(pub u64);

/// Opaque handle for a kernel-managed Grant region.
/// The value is the physical base address of the grant's first page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct GrantId(pub usize);

/// Access permission granted to a target cell for a Grant region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum GrantPerm {
    ReadOnly  = 0,
    WriteOnly = 1,
    ReadWrite = 2,
}

impl core::convert::TryFrom<u8> for GrantPerm {
    type Error = ();
    fn try_from(v: u8) -> Result<Self, ()> {
        match v {
            0 => Ok(Self::ReadOnly),
            1 => Ok(Self::WriteOnly),
            2 => Ok(Self::ReadWrite),
            _ => Err(()),
        }
    }
}

/// State of a Cell in its lifecycle.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellState {
    /// Cell is being loaded and linked.
    Loading,
    /// Cell is active and running.
    Active,
    /// Cell is marked for unload but still has references.
    Zombie,
    /// Cell is poisoned and is being recovered.
    Poisoned,
}

/// Semantic versioning for Cells.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SemVer {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl SemVer {
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

/// Physical memory address.
pub type PhysAddr = usize;

/// Virtual memory address (Renamed from VirtAddr for brevity & standardization).
pub type VAddr = usize;

/// Standard Result type for ViCell APIs.
pub type ViResult<T> = core::result::Result<T, ViError>;

/// Common error types.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViError {
    /// Out of memory.
    OutOfMemory,
    /// Invalid argument.
    InvalidArgument,
    /// Resource not found.
    NotFound,
    /// Permission denied.
    PermissionDenied,
    /// Resource already exists.
    AlreadyExists,
    /// Operation would block.
    WouldBlock,
    /// Operation not supported.
    NotSupported,
    /// I/O Error.
    IO,
    /// Invalid input data.
    InvalidInput,
    /// Is a directory.
    IsADirectory,
    /// Not a directory.
    NotADirectory,
    /// Unknown error.
    Unknown,
}

/// File Type Enum
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    File = 0,
    Directory = 1,
    Device = 2,
    Unknown = 255,
}

/// Directory Entry
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct DirEntry {
    pub name: [u8; 64], // Fixed size name
    pub file_type: FileType,
    pub size: u64,
}

impl Default for DirEntry {
    fn default() -> Self {
        Self {
            name: [0; 64],
            file_type: FileType::Unknown,
            size: 0,
        }
    }
}

pub mod silo;

// ─── Host-runnable unit tests ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // VAddr is usize — test that alignment helpers behave correctly.

    fn align_down(addr: VAddr, align: usize) -> VAddr {
        addr & !(align - 1)
    }

    fn align_up(addr: VAddr, align: usize) -> VAddr {
        (addr + align - 1) & !(align - 1)
    }

    #[test]
    fn vaddr_align_down_page() {
        assert_eq!(align_down(0x1000, 0x1000), 0x1000);
        assert_eq!(align_down(0x1001, 0x1000), 0x1000);
        assert_eq!(align_down(0x1fff, 0x1000), 0x1000);
        assert_eq!(align_down(0x2000, 0x1000), 0x2000);
    }

    #[test]
    fn vaddr_align_up_page() {
        assert_eq!(align_up(0x1000, 0x1000), 0x1000);
        assert_eq!(align_up(0x1001, 0x1000), 0x2000);
        assert_eq!(align_up(0x1fff, 0x1000), 0x2000);
        assert_eq!(align_up(0x2000, 0x1000), 0x2000);
    }

    #[test]
    fn vaddr_addition_does_not_overflow_in_range() {
        let base: VAddr = 0x8000_0000;
        let offset: usize = 0x1000;
        let result = base.wrapping_add(offset);
        assert_eq!(result, 0x8000_1000);
    }

    #[test]
    fn vaddr_subtraction_gives_offset() {
        let a: VAddr = 0x8000_2000;
        let b: VAddr = 0x8000_0000;
        assert_eq!(a - b, 0x2000);
    }

    #[test]
    fn semver_ordering() {
        let v100 = SemVer::new(1, 0, 0);
        let v110 = SemVer::new(1, 1, 0);
        let v111 = SemVer::new(1, 1, 1);
        assert!(v100 < v110);
        assert!(v110 < v111);
        assert_eq!(v100, SemVer::new(1, 0, 0));
    }

    #[test]
    fn cell_id_ordering() {
        let a = CellId(1);
        let b = CellId(2);
        assert!(a < b);
        assert_eq!(a, CellId(1));
    }

    #[test]
    fn dir_entry_default_is_zeroed() {
        let e = DirEntry::default();
        assert_eq!(e.size, 0);
        assert_eq!(e.name, [0u8; 64]);
        assert!(matches!(e.file_type, FileType::Unknown));
    }

    #[test]
    fn vi_error_variants_are_distinct() {
        assert_ne!(ViError::NotFound as u8, ViError::OutOfMemory as u8);
        assert_ne!(ViError::IO as u8, ViError::PermissionDenied as u8);
    }

    #[test]
    fn paddr_as_usize_round_trip() {
        let p: PhysAddr = 0xDEAD_BEEF;
        let v: VAddr = p; // both are usize aliases
        assert_eq!(v, 0xDEAD_BEEF);
    }

    #[test]
    fn page_size_alignment_properties() {
        const PAGE: usize = 0x1000;
        // Any page-aligned address aligns-up to itself.
        for base in [0usize, PAGE, PAGE * 3, PAGE * 255] {
            assert_eq!(align_up(base, PAGE), base);
            assert_eq!(align_down(base, PAGE), base);
        }
    }
}

