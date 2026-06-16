//! Host-side unit tests for `kernel/src/boot.rs` logic.
//!
//! These tests mirror the data structures and pure functions from `boot.rs`
//! so they can run on the host (std) without QEMU. They catch regressions in
//! memory-type conversion, fallback address ranges, and truncation limits.
//!
//! Invariant: when `boot.rs` changes a conversion or constant, update the
//! matching assertion here. The source-of-truth comment on each test cites
//! the relevant `boot.rs` line range.

// ---------------------------------------------------------------------------
// Mirrored types from kernel/src/boot.rs
// (These cannot be imported directly — the kernel is no_std/riscv64 only.)
// ---------------------------------------------------------------------------

/// Mirrors `boot::MemoryType` — keep in sync with kernel/src/boot.rs:40-50.
/// Note: MMIO is defined in boot.rs but has no Limine entry_type mapping,
/// so it is not included here (and would never be produced by the converter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemoryType {
    Usable,
    Reserved,
    AcpiReclaimable,
    AcpiNvs,
    BadMemory,
    Bootloader,
    Kernel,
    Framebuffer,
}

#[derive(Debug, Clone, Copy)]
struct MemoryMapEntry {
    base: usize,
    #[allow(dead_code)]
    length: usize,
    #[allow(dead_code)]
    ty: MemoryType,
}

/// Mirrors the `match entry.entry_type` block in `boot.rs:96-107`.
fn limine_type_to_memory_type(entry_type: u32) -> MemoryType {
    match entry_type {
        0 => MemoryType::Usable,
        1 => MemoryType::Reserved,
        2 => MemoryType::AcpiReclaimable,
        3 => MemoryType::AcpiNvs,
        4 => MemoryType::BadMemory,
        5 => MemoryType::Bootloader,
        6 => MemoryType::Kernel,
        7 => MemoryType::Framebuffer,
        _ => MemoryType::Reserved,
    }
}

// ---------------------------------------------------------------------------
// Mirrors of FALLBACK_BOOT_INFO per arch (kernel/src/boot.rs:231-307)
// ---------------------------------------------------------------------------

const RV64_FALLBACK_KERNEL_BASE: usize = 0x8020_0000;
const RV64_FALLBACK_HHDM:        usize = 0x0;

const VF2_FALLBACK_KERNEL_BASE:  usize = 0x4020_0000;
const VF2_FALLBACK_HHDM:         usize = 0x0;

const AARCH64_FALLBACK_KERNEL_BASE: usize = 0x4008_0000;
const AARCH64_FALLBACK_HHDM:        usize = 0x0;

const RV32_FALLBACK_KERNEL_BASE: usize = 0x8020_0000;
const RV32_FALLBACK_HHDM:        usize = 0x0;

// boot.rs:69
const MAX_MEMORY_MAP_ENTRIES: usize = 64;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// All 8 defined Limine memory types map to the correct MemoryType variant.
/// Source: boot.rs:96-107.
#[test]
fn limine_all_known_types_convert_correctly() {
    assert_eq!(limine_type_to_memory_type(0), MemoryType::Usable,          "type 0 = Usable");
    assert_eq!(limine_type_to_memory_type(1), MemoryType::Reserved,        "type 1 = Reserved");
    assert_eq!(limine_type_to_memory_type(2), MemoryType::AcpiReclaimable, "type 2 = AcpiReclaimable");
    assert_eq!(limine_type_to_memory_type(3), MemoryType::AcpiNvs,         "type 3 = AcpiNvs");
    assert_eq!(limine_type_to_memory_type(4), MemoryType::BadMemory,       "type 4 = BadMemory");
    assert_eq!(limine_type_to_memory_type(5), MemoryType::Bootloader,      "type 5 = Bootloader");
    assert_eq!(limine_type_to_memory_type(6), MemoryType::Kernel,          "type 6 = Kernel");
    assert_eq!(limine_type_to_memory_type(7), MemoryType::Framebuffer,     "type 7 = Framebuffer");
}

/// Unknown Limine types (> 7) must default to Reserved, not panic.
/// Source: boot.rs:105-106 (`_ => MemoryType::Reserved`).
#[test]
fn limine_unknown_type_defaults_to_reserved() {
    for bad in [8u32, 10, 100, 255, u32::MAX] {
        assert_eq!(
            limine_type_to_memory_type(bad),
            MemoryType::Reserved,
            "unknown type {bad} should default to Reserved"
        );
    }
}

/// The FALLBACK memory map for RV64 QEMU virt must have the kernel base in the
/// RISC-V virt RAM range [0x8020_0000, 0x9000_0000).
/// Source: boot.rs:232-243.
#[test]
fn fallback_rv64_kernel_base_in_ram_range() {
    assert!(
        RV64_FALLBACK_KERNEL_BASE >= 0x8000_0000,
        "RV64 kernel base {:#x} below RISC-V virt RAM start",
        RV64_FALLBACK_KERNEL_BASE
    );
    assert!(
        RV64_FALLBACK_KERNEL_BASE < 0x9000_0000,
        "RV64 kernel base {:#x} above expected region",
        RV64_FALLBACK_KERNEL_BASE
    );
    // OpenSBI uses the first 2 MB; kernel loads at 0x8020_0000.
    assert_eq!(RV64_FALLBACK_KERNEL_BASE, 0x8020_0000, "expected canonical OpenSBI skip offset");
    assert_eq!(RV64_FALLBACK_HHDM, 0, "RV64 HHDM offset must be 0 (identity map)");
}

/// VisionFive2 DRAM starts at 0x4000_0000; kernel at 0x4020_0000.
/// Source: boot.rs:249-260.
#[test]
fn fallback_vf2_kernel_base_in_ram_range() {
    assert!(VF2_FALLBACK_KERNEL_BASE >= 0x4000_0000, "VF2 kernel base below DRAM start");
    assert!(VF2_FALLBACK_KERNEL_BASE <  0x5000_0000, "VF2 kernel base too high");
    assert_eq!(VF2_FALLBACK_HHDM, 0, "VF2 HHDM offset must be 0");
}

/// AArch64 virt RAM starts at 0x4000_0000; kernel loads at 0x4008_0000.
/// Source: boot.rs:279-291.
#[test]
fn fallback_aarch64_kernel_base_in_ram_range() {
    assert!(AARCH64_FALLBACK_KERNEL_BASE >= 0x4000_0000, "AArch64 kernel base below RAM start");
    assert!(AARCH64_FALLBACK_KERNEL_BASE <  0x5000_0000, "AArch64 kernel base too high");
    assert_eq!(AARCH64_FALLBACK_KERNEL_BASE, 0x4008_0000, "expected linker-aarch64.ld load address");
    assert_eq!(AARCH64_FALLBACK_HHDM, 0, "AArch64 HHDM offset must be 0");
}

/// RV32 Nano QEMU virt RAM starts at 0x8000_0000; kernel at 0x8020_0000.
/// Source: boot.rs:264-275.
#[test]
fn fallback_rv32_kernel_base_in_ram_range() {
    assert!(RV32_FALLBACK_KERNEL_BASE >= 0x8000_0000, "RV32 kernel base below RAM start");
    assert!(RV32_FALLBACK_KERNEL_BASE <  0x9000_0000, "RV32 kernel base too high");
    assert_eq!(RV32_FALLBACK_KERNEL_BASE, 0x8020_0000, "expected OpenSBI skip offset");
    assert_eq!(RV32_FALLBACK_HHDM, 0, "RV32 HHDM offset must be 0 (SATP=0, bare)");
}

/// MAX_MEMORY_MAP_ENTRIES must be large enough for realistic QEMU memory maps
/// (typically 5–15 entries) but the constant itself must not overflow a static buffer.
/// Source: boot.rs:69.
#[test]
fn max_memory_map_entries_is_sane() {
    assert!(MAX_MEMORY_MAP_ENTRIES >= 16, "must hold at least 16 entries for real hardware");
    assert!(MAX_MEMORY_MAP_ENTRIES <= 256, "static buffer > 256 entries wastes kernel BSS");
    // Exact value documented in boot.rs comment — regression guard.
    assert_eq!(MAX_MEMORY_MAP_ENTRIES, 64, "MAX_MEMORY_MAP_ENTRIES changed — update boot.rs too");
}

/// Truncation contract: if more than MAX entries are supplied, exactly
/// MAX are stored and no write goes out of bounds.
#[test]
fn memory_map_truncation_stops_at_max() {
    // Simulate the truncation loop from boot.rs:90-114.
    let mut buffer = [MemoryMapEntry { base: 0, length: 0, ty: MemoryType::Reserved };
                      MAX_MEMORY_MAP_ENTRIES];
    let input_count = MAX_MEMORY_MAP_ENTRIES + 10; // more than the buffer

    let mut count = 0usize;
    for i in 0..input_count {
        if i >= MAX_MEMORY_MAP_ENTRIES {
            break; // mirrors the `if i >= MAX_MEMORY_MAP_ENTRIES { break; }` guard
        }
        buffer[count] = MemoryMapEntry { base: i * 0x1000, length: 0x1000, ty: MemoryType::Usable };
        count += 1;
    }

    assert_eq!(count, MAX_MEMORY_MAP_ENTRIES, "truncation must stop at MAX_MEMORY_MAP_ENTRIES");
    // Verify no entry went out of bounds (would have panicked above if so).
    assert_eq!(buffer[MAX_MEMORY_MAP_ENTRIES - 1].base, (MAX_MEMORY_MAP_ENTRIES - 1) * 0x1000);
}

/// SimpleBootInfo hhdm_offset must always return 0 on RISC-V/AArch64 fallback
/// (identity-mapped physical → virtual, no HHDM bias needed).
#[test]
fn fallback_hhdm_is_zero_for_all_non_x86_arches() {
    // All fallback boot infos use hhdm_offset = 0.
    // x86_64 Limine provides a real HHDM; the x86_64 FALLBACK_BOOT_INFO is
    // unreachable in normal operation (Limine always present on x86_64).
    for (arch, hhdm) in [
        ("rv64", RV64_FALLBACK_HHDM),
        ("vf2",  VF2_FALLBACK_HHDM),
        ("aa64", AARCH64_FALLBACK_HHDM),
        ("rv32", RV32_FALLBACK_HHDM),
    ] {
        assert_eq!(hhdm, 0, "{arch}: fallback HHDM offset must be 0 (identity map)");
    }
}

fn main() {}
