//! Identity-mapping IOMMU page tables for DMA isolation.
//!
//! Both table types map IOVA == PA for explicitly registered DMA ranges.
//! Any other IOVA causes an IOMMU fault — preventing arbitrary RAM access via DMA.
//!
//! `Sv39IommuPt` — RISC-V IOMMU second-stage (Sv39, 3 levels)
//! `VtdSlpt`     — Intel VT-d second-level page table (3 levels, AW=39-bit)

use alloc::alloc::{alloc_zeroed, Layout};

fn alloc_page() -> (usize, u64) {
    let layout = Layout::from_size_align(4096, 4096).expect("iommu_pt: page");
    // SAFETY: layout is non-zero and 4096-aligned.
    let ptr = unsafe { alloc_zeroed(layout) } as usize;
    assert!(ptr != 0, "[iommu_pt] OOM allocating IOMMU page");
    (ptr, virt_to_phys(ptr))
}

#[inline]
fn virt_to_phys(virt: usize) -> u64 {
    #[cfg(target_arch = "x86_64")]
    { (virt - crate::memory::frame::phys_to_virt(0)) as u64 }
    #[cfg(not(target_arch = "x86_64"))]
    { virt as u64 }
}

#[inline]
pub(super) fn phys_to_virt_inner(phys: u64) -> usize {
    #[cfg(target_arch = "x86_64")]
    { phys as usize + crate::memory::frame::phys_to_virt(0) }
    #[cfg(not(target_arch = "x86_64"))]
    { phys as usize }
}

// ── RISC-V Sv39 second-stage page table ──────────────────────────────────────

const SV39_V:   u64 = 1 << 0; // Valid
const SV39_R:   u64 = 1 << 1; // Read
const SV39_W:   u64 = 1 << 2; // Write
const SV39_A:   u64 = 1 << 6; // Accessed (pre-set to avoid IOMMU A-fault on first DMA)
const SV39_D:   u64 = 1 << 7; // Dirty   (pre-set to avoid IOMMU D-fault on first DMA)
/// Leaf DMA PTE: V|R|W|A|D — readable+writable, no execute.
const PTE_DMA:  u64 = SV39_V | SV39_R | SV39_W | SV39_A | SV39_D;

/// Sv39 3-level identity-mapping page table for RISC-V IOMMU second-stage.
pub struct Sv39IommuPt {
    root_phys: u64,
    root_virt: usize,
}

// SAFETY: pointers are into kernel-owned 4 KiB pages never aliased externally.
unsafe impl Send for Sv39IommuPt {}

impl Sv39IommuPt {
    pub fn new() -> Self {
        let (v, p) = alloc_page();
        Self { root_phys: p, root_virt: v }
    }

    /// Add an identity mapping (IOVA == PA) for [phys, phys+size). Idempotent.
    pub fn map_range(&self, phys: u64, size: usize) {
        let start = phys & !0xFFF;
        let end   = (phys + size as u64 + 0xFFF) & !0xFFF;
        let mut pa = start;
        while pa < end {
            self.map_page(pa);
            pa += 0x1000;
        }
    }

    fn map_page(&self, pa: u64) {
        let vpn2 = ((pa >> 30) & 0x1FF) as usize;
        let vpn1 = ((pa >> 21) & 0x1FF) as usize;
        let vpn0 = ((pa >> 12) & 0x1FF) as usize;
        let l1_phys = ensure_sv39_child(self.root_virt, vpn2);
        let l0_phys = ensure_sv39_child(phys_to_virt_inner(l1_phys), vpn1);
        let leaf = ((pa >> 12) << 10) | PTE_DMA;
        // SAFETY: l0 page is a valid 4 KiB allocation; vpn0 < 512.
        unsafe {
            let ptr = (phys_to_virt_inner(l0_phys) + vpn0 * 8) as *mut u64;
            ptr.write_volatile(leaf);
        }
    }

    /// Physical address of the root page (program into Device Context satp.PPN).
    #[inline]
    pub fn root_phys(&self) -> u64 { self.root_phys }
}

/// Get or allocate a child table at `table_virt[idx]`. Returns child phys.
fn ensure_sv39_child(table_virt: usize, idx: usize) -> u64 {
    // SAFETY: table_virt is a 512-entry 4 KiB page; idx < 512.
    let slot = (table_virt + idx * 8) as *mut u64;
    let e = unsafe { slot.read_volatile() };
    if e & SV39_V != 0 {
        return (e >> 10) << 12; // extract PPN → phys
    }
    let (_, child_phys) = alloc_page();
    let ptr_pte = ((child_phys >> 12) << 10) | SV39_V; // V=1, no R/W/X = non-leaf
    unsafe { slot.write_volatile(ptr_pte); }
    child_phys
}

// ── Intel VT-d 3-level SLPT (AW=39-bit) ──────────────────────────────────────

/// VT-d SLPT entry flag: R=1|W=1 required for all valid entries (leaf + non-leaf).
const VTD_RW: u64 = 0b11;

/// Intel VT-d second-level page table (3 levels, 39-bit address width).
pub struct VtdSlpt {
    root_phys: u64,
    root_virt: usize,
}

// SAFETY: pointers are into kernel-owned 4 KiB pages never aliased externally.
unsafe impl Send for VtdSlpt {}

impl VtdSlpt {
    pub fn new() -> Self {
        let (v, p) = alloc_page();
        Self { root_phys: p, root_virt: v }
    }

    /// Add an identity mapping (IOVA == PA) for [phys, phys+size). Idempotent.
    pub fn map_range(&self, phys: u64, size: usize) {
        let start = phys & !0xFFF;
        let end   = (phys + size as u64 + 0xFFF) & !0xFFF;
        let mut pa = start;
        while pa < end {
            self.map_page(pa);
            pa += 0x1000;
        }
    }

    fn map_page(&self, pa: u64) {
        let i2 = ((pa >> 30) & 0x1FF) as usize;
        let i1 = ((pa >> 21) & 0x1FF) as usize;
        let i0 = ((pa >> 12) & 0x1FF) as usize;
        let l1_phys = ensure_vtd_child(self.root_virt, i2);
        let l0_phys = ensure_vtd_child(phys_to_virt_inner(l1_phys), i1);
        let leaf = (pa & !0xFFF) | VTD_RW;
        // SAFETY: l0 page is a valid 4 KiB allocation; i0 < 512.
        unsafe {
            let ptr = (phys_to_virt_inner(l0_phys) + i0 * 8) as *mut u64;
            ptr.write_volatile(leaf);
        }
    }

    /// Physical address of the SLPT root page.
    #[inline]
    pub fn root_phys(&self) -> u64 { self.root_phys }
}

/// Get or allocate a child table at `table_virt[idx]`. Returns child phys.
fn ensure_vtd_child(table_virt: usize, idx: usize) -> u64 {
    // SAFETY: table_virt is a 512-entry 4 KiB page; idx < 512.
    let slot = (table_virt + idx * 8) as *mut u64;
    let e = unsafe { slot.read_volatile() };
    if e & VTD_RW != 0 {
        return e & !0xFFF; // extract physical address from non-leaf entry
    }
    let (_, child_phys) = alloc_page();
    unsafe { slot.write_volatile((child_phys & !0xFFF) | VTD_RW); }
    child_phys
}
