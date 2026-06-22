//! IOMMU common API — three-phase DMA isolation.
//!
//! Phase 1 `init()`             — probe hardware, allocate page tables, stay passthrough.
//! Phase 2 `map_dma()`          — drivers register each DMA buffer's physical range.
//! Phase 3 `activate_isolation()` — switch from passthrough to enforced page-table mode.
//!
//! Call order in `main.rs`:
//!   `iommu::init()` → driver DMA allocs (call `map_dma()`) → `iommu::activate_isolation()`

use core::sync::atomic::{AtomicBool, Ordering};

static IOMMU_ISOLATED: AtomicBool = AtomicBool::new(false);

/// Phase 1: probe IOMMU hardware and allocate isolation data structures.
///
/// Must be called after `pcie_ecam::init()` and before any DMA allocation.
/// Does NOT enable enforcement yet — hardware stays in passthrough mode.
pub fn init() {
    #[cfg(target_arch = "riscv64")]
    super::iommu_riscv::init_hw();
    #[cfg(target_arch = "x86_64")]
    super::iommu_x86::init_hw();
}

/// Phase 2: register a DMA physical range in the IOMMU page table.
///
/// Must be called for every DMA buffer allocated between `init()` and
/// `activate_isolation()`. Returns the IOVA (identity == phys for SAS).
#[inline]
pub fn map_dma(phys: u64, size: usize) -> u64 {
    if size == 0 { return phys; }
    #[cfg(target_arch = "riscv64")]
    super::iommu_riscv::map_range(phys, size);
    #[cfg(target_arch = "x86_64")]
    super::iommu_x86::map_range(phys, size);
    phys
}

/// No-op stub. Dynamic IOTLB invalidation (needed for full unmap) is G2 work.
#[inline]
pub fn unmap_dma(_iova: u64, _size: usize) {}

/// Phase 3: switch IOMMU from passthrough to page-table enforcement.
///
/// On RISC-V: writes DDTP with MODE=1LVL + pre-built Sv39 DDT → faults any
///   IOVA not in a registered DMA range.
/// On x86_64: fills VT-d context entries with TT=TRANSLATED+SLPT, enables TE.
///
/// Call after all driver DMA buffers are registered via `map_dma()`.
pub fn activate_isolation() {
    #[cfg(target_arch = "riscv64")]
    super::iommu_riscv::activate();
    #[cfg(target_arch = "x86_64")]
    super::iommu_x86::activate();
}

/// Returns `true` once `activate_isolation()` has completed successfully.
#[inline]
pub fn is_active() -> bool {
    IOMMU_ISOLATED.load(Ordering::Relaxed)
}

/// Mark DMA isolation as active. Called by arch backends on successful activation.
pub(super) fn set_active() {
    IOMMU_ISOLATED.store(true, Ordering::Relaxed);
}
