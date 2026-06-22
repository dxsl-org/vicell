//! RISC-V IOMMU driver — DMA isolation via 1-level DDT + Sv39 second-stage.
//!
//! Phase 1 `init_hw()`:   probe PCIe IOMMU device, allocate 64-entry DDT and
//!                         Sv39 second-stage page table. Stays in BARE mode.
//! Phase 2 `map_range()`: add a physical range to the Sv39 page table.
//! Phase 3 `activate()`:  fill all DDT entries → switch DDTP to MODE=1LVL.
//!
//! After activation every IOVA not in a registered DMA range causes an
//! IOMMU fault, preventing arbitrary RAM access via DMA.

use alloc::alloc::{alloc_zeroed, Layout};
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use crate::sync::Spinlock;
use super::iommu_pt::Sv39IommuPt;
use crate::task::drivers::pcie_ecam;

// PCIe identification (RISC-V IOMMU spec §3 + QEMU hw/riscv/riscv-iommu.c)
const CLASS:  u8 = 0x08;
const SUB:    u8 = 0x06;
const PROGIF: u8 = 0x00;

// BAR0 register offsets (RISC-V IOMMU spec v1.0 §3.1)
const REG_CAPS: usize = 0x00; // 64-bit capabilities
const REG_FCTL: usize = 0x08; // 32-bit feature control
const REG_DDTP: usize = 0x10; // 64-bit device-directory-table pointer
const REG_IPSR: usize = 0x38; // 32-bit interrupt-pending status

// DDTP.MODE values (bits [3:0])
const DDTP_MODE_BARE: u64 = 1; // passthrough — IOVA == PA, no enforcement
const DDTP_MODE_1LVL: u64 = 2; // 1-level DDT, 64 DCs indexed by DeviceID[5:0]

// Device Context (DC) field constants
const DC_TC_V:        u64 = 1;          // TC.V=1 — context is valid
const SATP_MODE_SV39: u64 = 8u64 << 60; // Sv39 page-table mode for satp field

// ── Module-level state ────────────────────────────────────────────────────────

static BAR0:     AtomicUsize = AtomicUsize::new(0);
static DDT_VIRT: AtomicUsize = AtomicUsize::new(0);
static DDT_PHYS: AtomicU64   = AtomicU64::new(0);
static RISCV_PT: Spinlock<Option<Sv39IommuPt>> = Spinlock::new(None);

// ── MMIO helpers ─────────────────────────────────────────────────────────────

#[inline]
unsafe fn read32(base: usize, off: usize) -> u32 {
    // SAFETY: caller ensures base is valid identity-mapped MMIO.
    unsafe { core::ptr::read_volatile((base + off) as *const u32) }
}
#[inline]
unsafe fn write32(base: usize, off: usize, val: u32) {
    // SAFETY: caller ensures base is valid identity-mapped MMIO.
    unsafe { core::ptr::write_volatile((base + off) as *mut u32, val) }
}
#[inline]
unsafe fn write64(base: usize, off: usize, val: u64) {
    // SAFETY: caller ensures base is valid identity-mapped MMIO.
    unsafe { core::ptr::write_volatile((base + off) as *mut u64, val) }
}

// ── Phase 1: probe + allocate ─────────────────────────────────────────────────

/// Probe RISC-V IOMMU hardware and allocate DDT + Sv39 page table.
/// Stays in BARE (passthrough) mode until `activate()` is called.
pub(super) fn init_hw() {
    let dev = match pcie_ecam::find_class(CLASS, SUB, PROGIF) {
        Some(d) => d,
        None => {
            log::warn!("[iommu] RISC-V IOMMU not found \
                        (needs QEMU ≥ 8.2 + -device riscv-iommu-pci,bus=pcie.0)");
            return;
        }
    };
    let bar0 = dev.bars[0].base_addr() as usize;
    if bar0 == 0 {
        log::warn!("[iommu] RISC-V IOMMU BAR0 == 0");
        return;
    }

    // Read caps (unused for now; future: check supported DDTP modes).
    let _caps = unsafe { core::ptr::read_volatile((bar0 + REG_CAPS) as *const u64) };
    unsafe {
        write32(bar0, REG_FCTL, 0);              // little-endian, no command FIFO
        write64(bar0, REG_DDTP, DDTP_MODE_BARE); // stay passthrough
        let ipsr = read32(bar0, REG_IPSR);
        if ipsr != 0 { write32(bar0, REG_IPSR, ipsr); } // clear pending faults (W1C)
    }

    // Allocate 1-level DDT: 64 entries × 64 B = 4096 B, 4096-aligned.
    let layout = Layout::from_size_align(4096, 4096).expect("riscv iommu: DDT");
    let ddt_virt = unsafe { alloc_zeroed(layout) } as usize;
    assert!(ddt_virt != 0, "[iommu_riscv] OOM: DDT");
    let ddt_phys = ddt_virt as u64; // identity-mapped on RISC-V: VA == PA

    BAR0.store(bar0, Ordering::Relaxed);
    DDT_VIRT.store(ddt_virt, Ordering::Relaxed);
    DDT_PHYS.store(ddt_phys, Ordering::Relaxed);
    *RISCV_PT.lock() = Some(Sv39IommuPt::new());

    log::info!("[iommu] RISC-V IOMMU HW ready (vendor={:04x} dev={:04x}) \
                — isolation pending", dev.vendor_id, dev.device_id);
}

// ── Phase 2: register DMA range ───────────────────────────────────────────────

/// Add [phys, phys+size) to the Sv39 second-stage page table.
pub(super) fn map_range(phys: u64, size: usize) {
    if let Some(pt) = RISCV_PT.lock().as_ref() {
        pt.map_range(phys, size);
    }
}

// ── Phase 3: activate enforcement ────────────────────────────────────────────

/// Switch DDTP from BARE to 1-level DDT with Sv39 second-stage enforcement.
///
/// After this call, DMA from any PCIe device is restricted to ranges registered
/// via `map_range()`. All other IOVAs trigger an IOMMU fault.
pub(super) fn activate() {
    let bar0     = BAR0.load(Ordering::Relaxed);
    let ddt_virt = DDT_VIRT.load(Ordering::Relaxed);
    let ddt_phys = DDT_PHYS.load(Ordering::Relaxed);
    if bar0 == 0 || ddt_virt == 0 { return; } // IOMMU not present

    let pt_root = match RISCV_PT.lock().as_ref() {
        Some(pt) => pt.root_phys(),
        None => return,
    };

    // satp field of Device Context: Sv39, ASID=0, PPN=pt_root/4096.
    let fsc = SATP_MODE_SV39 | (pt_root >> 12);

    // Populate all 64 DDT entries with a valid Device Context pointing to the
    // shared Sv39 page table. DeviceID[5:0] indexes the entry (i = DC index).
    for i in 0usize..64 {
        let dc = ddt_virt + i * 64;
        // SAFETY: dc is within the 4096-byte DDT allocation; fields are u64.
        unsafe {
            core::ptr::write_volatile((dc     ) as *mut u64, DC_TC_V); // tc: valid
            core::ptr::write_volatile((dc +  8) as *mut u64, 0u64);    // gatp: bare G-stage
            core::ptr::write_volatile((dc + 16) as *mut u64, 0u64);    // ta: default
            core::ptr::write_volatile((dc + 24) as *mut u64, fsc);     // satp: Sv39
            core::ptr::write_volatile((dc + 32) as *mut u64, 0u64);    // msiptp: none
            core::ptr::write_volatile((dc + 40) as *mut u64, 0u64);    // msi_addr_mask
            core::ptr::write_volatile((dc + 48) as *mut u64, 0u64);    // msi_addr_pattern
            core::ptr::write_volatile((dc + 56) as *mut u64, 0u64);    // reserved
        }
    }

    // Switch DDTP to 1LVL mode — DMA enforcement is now active.
    // DDTP[3:0]=MODE, DDTP[63:10]=PPN (physical page number of DDT).
    let ddtp = ((ddt_phys >> 12) << 10) | DDTP_MODE_1LVL;
    unsafe { write64(bar0, REG_DDTP, ddtp); }

    super::iommu::set_active();
    log::info!("[iommu] RISC-V IOMMU: DMA isolation ACTIVE (Sv39 second-stage, 1LVL DDT)");
}
