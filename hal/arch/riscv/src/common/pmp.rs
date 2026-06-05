//! RISC-V PMP (Physical Memory Protection) region definitions.
//!
//! # Architecture Constraint
//!
//! PMP CSRs (`pmpaddr*`, `pmpcfg*`) are **M-mode-only**.  Writing them from
//! S-mode raises an Illegal Instruction trap.  ViCell runs in S-mode under
//! OpenSBI, so this module **cannot write PMP registers at runtime**.
//!
//! This file documents the intended PMP layout for use by a future custom
//! M-mode firmware shim (Phase 32+).  The `KERNEL_PMP_REGIONS` slice encodes
//! the desired configuration that the shim should apply before `mret` into
//! S-mode.
//!
//! # QEMU virt PMP budget
//! - 16 total entries; OpenSBI claims 2 (entry 0 = firmware deny, entry 1 = all-access)
//! - 14 free entries for ViCell (entries 2–15)
//! - Entries 2–6 used for kernel + MMIO protection (see KERNEL_PMP_REGIONS)
//! - Entries 7–15 reserved for per-Cell isolation (Phase 32)
//!
//! # NAPOT encoding
//! `pmpaddr = (base >> 2) | (size/8 - 1)`
//! Requirements: `base` aligned to `size`; `size` must be a power of two ≥ 8.

/// Permission bits for PMP config entries (`pmpcfg` byte per region).
pub mod perm {
    /// Read permission.
    pub const R: u8 = 0b001;
    /// Write permission.
    pub const W: u8 = 0b010;
    /// Execute permission.
    pub const X: u8 = 0b100;
    /// Read + Write.
    pub const RW: u8 = R | W;
    /// Read + Execute.
    pub const RX: u8 = R | X;
    /// Read + Write + Execute.
    pub const RWX: u8 = R | W | X;
    /// Addressing mode: NAPOT (naturally aligned power-of-two).
    pub const A_NAPOT: u8 = 0b11 << 3;
    /// Lock bit: entry enforced on M-mode too; cannot be modified until reset.
    /// Under Smepmp with MML=1, locked entries become M-mode-only rules.
    pub const L: u8 = 1 << 7;
}

/// Compute the NAPOT `pmpaddr` value for a region.
///
/// `base` must be aligned to `size`; `size` must be a power of two ≥ 8.
pub const fn napot_addr(base: usize, size: usize) -> usize {
    (base >> 2) | (size / 8 - 1)
}

/// Descriptor for a single PMP region (consumed by M-mode firmware table).
#[derive(Clone, Copy, Debug)]
pub struct PmpRegion {
    /// Physical base address (NAPOT-aligned to `size`).
    pub base: usize,
    /// Region size in bytes (must be a power of two ≥ 8).
    pub size: usize,
    /// Permission byte: R/W/X flags + addressing mode + optional lock.
    pub perms: u8,
}

/// Intended kernel protection regions for entries 2–6.
///
/// Entries 0–1 are owned by OpenSBI.  These descriptors should be written
/// to PMP entries 2–6 by the M-mode firmware shim before entering S-mode.
///
/// Physical addresses are for the QEMU virt machine with ViCell's default
/// linker layout (`ORIGIN = 0x80200000`).  Adjust if the layout changes.
pub const KERNEL_PMP_REGIONS: &[PmpRegion] = &[
    // Kernel .text + .rodata: read + execute, locked.
    // Prevents Cells from writing to kernel code via a wild physical pointer.
    PmpRegion {
        base:  0x8020_0000,
        size:  4 * 1024 * 1024, // 4 MiB — covers typical kernel binary
        perms: perm::RX | perm::A_NAPOT | perm::L,
    },
    // Kernel .data + .bss + stack: read + write, locked.
    // W^X: data region is not executable.
    PmpRegion {
        base:  0x8060_0000,
        size:  4 * 1024 * 1024,
        perms: perm::RW | perm::A_NAPOT | perm::L,
    },
    // VirtIO MMIO range (UART, VirtIO block/net/keyboard/gpu).
    PmpRegion {
        base:  0x1000_0000,
        size:  65536, // 64 KiB — covers UART + 8 VirtIO MMIO slots
        perms: perm::RW | perm::A_NAPOT,
    },
    // PLIC (Platform-Level Interrupt Controller).
    PmpRegion {
        base:  0x0C00_0000,
        size:  16 * 1024 * 1024, // 16 MiB
        perms: perm::RW | perm::A_NAPOT,
    },
    // CLINT (Core-Local Interrupt: mtime, msip).
    PmpRegion {
        base:  0x0200_0000,
        size:  65536,
        perms: perm::RW | perm::A_NAPOT,
    },
];
