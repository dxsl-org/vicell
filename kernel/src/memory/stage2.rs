//! ARM64 Stage-2 (IPA → PA) page-table builder for the ViCell hypervisor.
//!
//! # Layout
//! 40-bit IPA space, 4 KB granule, 3-level walk starting at Level 1 (VTCR.SL0=1).
//! Root = 2 × 512-entry Level-1 frames (8 KB, 8 KB-aligned); VMID ≥ 1.
//!
//! # Invariants (enforced)
//! - Guest RAM IPA range maps exclusively to the carved HPA region (SAS isolation).
//! - Emulated-MMIO IPAs (PL011, GIC, virtio-mmio ×4) are intentionally **unmapped**
//!   so Alpine probes trap to the hypervisor cell (ISV=0 mitigation).
//! - `Drop` frees every frame allocated here — no leak on VM teardown (Law 8).
//!
//! # Descriptor format (ARM DDI 0487 D8.3)
//! | Field | Bits | Value used |
//! |-------|------|-----------|
//! | Valid | [0] | 1 |
//! | Table/Block | [1] | 1 = table/page; 0 = block |
//! | MemAttr | [5:2] | 0b1111 = Normal Inner+Outer WB-WA |
//! | S2AP | [7:6] | 0b11 = RW, 0b01 = RO |
//! | SH | [9:8] | 0b11 = Inner-shareable |
//! | AF | [10] | 1 (suppress AF-fault) |
//! | Output PA | [47:12] | host physical frame address |

extern crate alloc;
use alloc::vec::Vec;

use super::frame::{allocate_guest_ram, phys_to_virt, FRAME_ALLOCATOR};
use super::paging::PAGE_SIZE;

// ── S2 descriptor constants ─────────────────────────────────────────────────

const DESC_VALID:  u64 = 1 << 0;
const DESC_TABLE:  u64 = 1 << 1; // At L1/L2 → table pointer; at L3 → page descriptor

// Stage-2 MemAttr bits[5:2] — inline, not a MAIR index.
const S2_MEMATTR_NORMAL: u64 = 0b1111 << 2; // Normal Inner+Outer WB-WA
const S2_S2AP_RW:        u64 = 0b11   << 6; // Read-write
const S2_S2AP_RO:        u64 = 0b01   << 6; // Read-only
const S2_SH_INNER:       u64 = 0b11   << 8; // Inner-shareable
const S2_AF:             u64 =    1   << 10; // Access Flag (suppress fault)

// Base flags for a Normal-WB-IS entry (MemAttr | S2AP_RW | SH | AF).
const S2_BASE_RW: u64 = S2_MEMATTR_NORMAL | S2_S2AP_RW | S2_SH_INNER | S2_AF;
const S2_BASE_RO: u64 = S2_MEMATTR_NORMAL | S2_S2AP_RO | S2_SH_INNER | S2_AF;

// PA mask: bits[47:12].
const PA_MASK: u64 = 0x0000_FFFF_FFFF_F000;

// ── Address-space constants ─────────────────────────────────────────────────

// 40-bit IPA space: T0SZ=24 → 2^40 = 1 TiB.
const IPA_LIMIT: u64 = 1 << 40;

// Translation level shifts for 4 KB granule.
const L1_SHIFT: u32 = 30; // L1: bits[39:30], 1 GB per entry
const L2_SHIFT: u32 = 21; // L2: bits[29:21], 2 MB per entry
const L3_SHIFT: u32 = 12; // L3: bits[20:12], 4 KB per entry

// 9-bit index within one 512-entry table.
const IDX_MASK: u64 = 0x1FF;

// Concatenated L1 index: 10 bits (1024 entries across 2 × L1 frames).
const L1_CONC_MASK: u64 = 0x3FF;

// ── MMIO holes that must remain unmapped before HCR_EL2.VM=1 (M3) ───────────

/// IPA ranges left **unmapped** so guest device-probe traps reach the hypervisor.
/// Frozen before `enable_stage2` is called; never remapped post-activation
/// without a full S2 TLB invalidation.
pub const MMIO_HOLES: &[(u64, u64)] = &[
    (0x08000000, 0x08020000), // GIC distributor + CPU interface (GICD/GICC)
    (0x09000000, 0x09001000), // PL011 UART
    (0x0a000000, 0x0a004000), // virtio-mmio bus: 4 slots × 0x1000 (P06/P07/P08)
];

// ── Helpers ──────────────────────────────────────────────────────────────────

#[inline]
fn l1_idx(ipa: u64) -> usize { ((ipa >> L1_SHIFT) & L1_CONC_MASK) as usize }
#[inline]
fn l2_idx(ipa: u64) -> usize { ((ipa >> L2_SHIFT) & IDX_MASK) as usize }
#[inline]
fn l3_idx(ipa: u64) -> usize { ((ipa >> L3_SHIFT) & IDX_MASK) as usize }

#[inline]
fn desc_pa(desc: u64) -> usize { (desc & PA_MASK) as usize }

#[inline]
fn table_desc(next_pa: u64) -> u64 { (next_pa & PA_MASK) | DESC_TABLE | DESC_VALID }

#[inline]
fn page_desc(pa: u64, writable: bool) -> u64 {
    // At L3, bits[1:0]=0b11 → page descriptor (DESC_TABLE | DESC_VALID).
    let base = if writable { S2_BASE_RW } else { S2_BASE_RO };
    (pa & PA_MASK) | base | DESC_TABLE | DESC_VALID
}

#[inline]
fn page_desc_device(pa: u64, writable: bool) -> u64 {
    // Device-nGnRnE: MemAttr[5:2]=0b0000, SH[9:8]=non-shareable=0b00, AF=1.
    let ap = if writable { S2_S2AP_RW } else { S2_S2AP_RO };
    (pa & PA_MASK) | ap | S2_AF | DESC_TABLE | DESC_VALID
}

// ── Error type ───────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
pub enum S2MapError {
    /// IPA or HPA range wraps around (checked_add overflow, C3).
    Overflow,
    /// IPA or HPA exceeds the 40-bit PA space limit.
    OutOfBounds,
    /// HPA is outside the guest's carved RAM region (SAS isolation).
    SasViolation,
    /// A block descriptor already occupies a slot we need to subdivide.
    BlockConflict,
    /// Frame allocator exhausted.
    OutOfMemory,
    /// Attempt to map a reserved MMIO hole (M3).
    MmioHole,
}

// ── Stage2Table ──────────────────────────────────────────────────────────────

/// ARM64 Stage-2 page table for one VM.
///
/// **Root alignment:** the two concatenated Level-1 frames occupy 8 KB starting
/// at `root_pa`, which must be 8 KB-aligned for VTTBR_EL2.BADDR.
///
/// **Safety:** the raw pointer fields are valid for as long as `self` lives.
/// The pointed-to frames are freed in `Drop`.  Never share across threads without
/// external synchronisation.
pub struct Stage2Table {
    // Root: 2 × 512-entry L1 frames at root_pa (8 KB, 8 KB-aligned).
    root_pa: u64,
    root_va: *mut u64, // maps to 1024 entries (2 × 512)

    // L2/L3 sub-table frames allocated on demand.  Tracked for Drop.
    sub_frames: Vec<usize>, // physical addresses

    // Carved guest-RAM region used for SAS-isolation assertion in map().
    guest_ram_pa:    u64,
    guest_ram_pages: usize,
}

// SAFETY: Stage2Table is not Send/Sync by default because of *mut u64.
// It is only accessed from single-CPU kernel context in QEMU TCG Phase 02/03.
unsafe impl Send for Stage2Table {}

impl Stage2Table {
    /// Allocate a new Stage-2 root (2 × 4 KB frames, 8 KB-aligned).
    ///
    /// Returns `None` when the frame allocator has fewer than 2 contiguous
    /// free frames.
    pub fn new() -> Option<Self> {
        // allocate_contiguous(2) guarantees contiguous and 4 KB alignment;
        // for 8 KB alignment we rely on the allocator returning an even-indexed
        // frame pair.  The FrameAllocator's memory_start is page-aligned, so
        // every pair at an even index is 8 KB-aligned.
        let root_pa = {
            let mut g = FRAME_ALLOCATOR.lock();
            g.as_mut()?.allocate_contiguous(2)? as u64
        };
        debug_assert_eq!(root_pa % (2 * PAGE_SIZE as u64), 0, "S2 root not 8KB-aligned");

        let root_va = phys_to_virt(root_pa as usize) as *mut u64;
        // Zero both L1 frames (1024 entries).
        // SAFETY: we just allocated these frames; they are ours exclusively.
        unsafe { core::ptr::write_bytes(root_va, 0, 1024); }

        Some(Self {
            root_pa,
            root_va,
            sub_frames: Vec::new(),
            guest_ram_pa: 0,
            guest_ram_pages: 0,
        })
    }

    /// Physical address of the root frame (for VTTBR_EL2.BADDR).
    #[inline]
    pub fn root_pa(&self) -> u64 { self.root_pa }

    /// Carve `n_pages` contiguous physical frames for guest RAM.
    ///
    /// Records the carved region so `map()` can enforce the SAS-isolation
    /// invariant (guest Stage-2 may only map into this region).
    ///
    /// Uses `allocate_guest_ram` (chunked scan, M2-mitigated).
    pub fn carve_guest_ram(&mut self, n_pages: usize) -> Option<u64> {
        let pa = allocate_guest_ram(n_pages)? as u64;
        self.guest_ram_pa = pa;
        self.guest_ram_pages = n_pages;
        Some(pa)
    }

    // ── Internal table allocator ─────────────────────────────────────────────

    fn alloc_subtable(&mut self) -> Option<(*mut u64, u64)> {
        let pa = {
            let mut g = FRAME_ALLOCATOR.lock();
            g.as_mut()?.allocate_frame()?
        };
        let va = phys_to_virt(pa) as *mut u64;
        // SAFETY: freshly allocated frame is exclusively ours.
        unsafe { core::ptr::write_bytes(va, 0, 512); }
        self.sub_frames.push(pa);
        Some((va, pa as u64))
    }

    // ── Public map / unmap ───────────────────────────────────────────────────

    /// Map `n_pages` × 4 KB at guest IPA `ipa` → host PA `hpa`.
    ///
    /// # C3 — overflow guard
    /// Both `ipa + n_pages × PAGE_SIZE` and `hpa + n_pages × PAGE_SIZE` are
    /// checked with `checked_add`; a wrapping range returns `Overflow`.
    ///
    /// # M3 — MMIO-hole guard
    /// Any IPA that overlaps a reserved MMIO hole returns `MmioHole`.
    ///
    /// # SAS-isolation guard
    /// If a guest-RAM region was carved via `carve_guest_ram`, `hpa` must
    /// lie within it; otherwise `SasViolation` is returned.
    pub fn map(
        &mut self,
        ipa: u64,
        hpa: u64,
        n_pages: usize,
        writable: bool,
    ) -> Result<(), S2MapError> {
        let page_bytes = n_pages as u64 * PAGE_SIZE as u64;

        // C3: overflow guards.
        let ipa_end = ipa.checked_add(page_bytes).ok_or(S2MapError::Overflow)?;
        let hpa_end = hpa.checked_add(page_bytes).ok_or(S2MapError::Overflow)?;

        if ipa_end > IPA_LIMIT { return Err(S2MapError::OutOfBounds); }
        if hpa_end > IPA_LIMIT { return Err(S2MapError::OutOfBounds); }

        // M3: reject any mapping that touches a reserved MMIO hole.
        for &(hole_base, hole_end) in MMIO_HOLES {
            if ipa < hole_end && ipa_end > hole_base {
                return Err(S2MapError::MmioHole);
            }
        }

        // SAS isolation: if guest RAM is known, HPA must stay within it.
        if self.guest_ram_pages > 0 {
            let guest_end = self.guest_ram_pa
                + (self.guest_ram_pages as u64 * PAGE_SIZE as u64);
            if hpa < self.guest_ram_pa || hpa_end > guest_end {
                return Err(S2MapError::SasViolation);
            }
        }

        let mut cur_ipa = ipa;
        let mut cur_hpa = hpa;
        for _ in 0..n_pages {
            self.map_single(cur_ipa, cur_hpa, writable)?;
            cur_ipa += PAGE_SIZE as u64;
            cur_hpa += PAGE_SIZE as u64;
        }
        Ok(())
    }

    /// Unmap `n_pages` pages starting at guest IPA `ipa`.
    ///
    /// Silently skips pages that are not mapped.  Does NOT free L2/L3 sub-tables
    /// even if they become fully empty — deferred to `Drop`.
    pub fn unmap(&mut self, ipa: u64, n_pages: usize) {
        let mut cur = ipa;
        for _ in 0..n_pages {
            self.unmap_single(cur);
            cur += PAGE_SIZE as u64;
        }
    }

    /// Map MMIO HPA directly into guest IPA space for hardware passthrough.
    ///
    /// Bypasses both the MMIO-hole guard and the SAS-isolation guard.  Use only for
    /// legitimate hardware passthrough (e.g., GICV HPA → GICC IPA for vGIC).
    /// Uses Device-nGnRnE memory attributes.
    ///
    /// # Errors
    /// Returns `Overflow` on wraparound; `OutOfBounds` if range exceeds 40-bit IPA limit.
    pub fn map_mmio_passthrough(
        &mut self,
        ipa: u64,
        hpa: u64,
        n_pages: usize,
        writable: bool,
    ) -> Result<(), S2MapError> {
        let page_bytes = n_pages as u64 * PAGE_SIZE as u64;
        let ipa_end = ipa.checked_add(page_bytes).ok_or(S2MapError::Overflow)?;
        let hpa_end = hpa.checked_add(page_bytes).ok_or(S2MapError::Overflow)?;
        if ipa_end > IPA_LIMIT { return Err(S2MapError::OutOfBounds); }
        if hpa_end > IPA_LIMIT { return Err(S2MapError::OutOfBounds); }
        let mut cur_ipa = ipa;
        let mut cur_hpa = hpa;
        for _ in 0..n_pages {
            self.map_single_device(cur_ipa, cur_hpa, writable)?;
            cur_ipa += PAGE_SIZE as u64;
            cur_hpa += PAGE_SIZE as u64;
        }
        Ok(())
    }

    // ── Single-page walk helpers ─────────────────────────────────────────────

    fn map_single(&mut self, ipa: u64, hpa: u64, writable: bool) -> Result<(), S2MapError> {
        // ── Level 1 ──────────────────────────────────────────────────────────
        let l1_ptr: *mut u64 =
            // SAFETY: root_va valid for 1024 entries; l1_idx < 1024.
            unsafe { self.root_va.add(l1_idx(ipa)) };
        let l1e = unsafe { *l1_ptr };

        let l2_va: *mut u64 = if l1e & (DESC_VALID | DESC_TABLE) == (DESC_VALID | DESC_TABLE) {
            // Existing L2 table pointer.
            phys_to_virt(desc_pa(l1e)) as *mut u64
        } else if l1e & DESC_VALID == 0 {
            // Allocate a fresh L2 table.
            let (va, pa) = self.alloc_subtable().ok_or(S2MapError::OutOfMemory)?;
            // SAFETY: l1_ptr valid; DESC_TABLE|DESC_VALID cannot overlap PA bits.
            unsafe { *l1_ptr = table_desc(pa); }
            va
        } else {
            return Err(S2MapError::BlockConflict); // L1 block — refuse to split
        };

        // ── Level 2 ──────────────────────────────────────────────────────────
        let l2_ptr: *mut u64 =
            // SAFETY: l2_va valid for 512 entries; l2_idx < 512.
            unsafe { l2_va.add(l2_idx(ipa)) };
        let l2e = unsafe { *l2_ptr };

        let l3_va: *mut u64 = if l2e & (DESC_VALID | DESC_TABLE) == (DESC_VALID | DESC_TABLE) {
            phys_to_virt(desc_pa(l2e)) as *mut u64
        } else if l2e & DESC_VALID == 0 {
            let (va, pa) = self.alloc_subtable().ok_or(S2MapError::OutOfMemory)?;
            // SAFETY: l2_ptr valid.
            unsafe { *l2_ptr = table_desc(pa); }
            va
        } else {
            return Err(S2MapError::BlockConflict); // L2 block
        };

        // ── Level 3 (page descriptor) ─────────────────────────────────────────
        let l3_ptr: *mut u64 =
            // SAFETY: l3_va valid for 512 entries; l3_idx < 512.
            unsafe { l3_va.add(l3_idx(ipa)) };
        // SAFETY: writing a well-formed page descriptor.
        unsafe { *l3_ptr = page_desc(hpa, writable); }

        Ok(())
    }

    /// Same L1→L2→L3 walk as `map_single`, but writes a Device-nGnRnE L3 descriptor.
    fn map_single_device(&mut self, ipa: u64, hpa: u64, writable: bool) -> Result<(), S2MapError> {
        let l1_ptr: *mut u64 = unsafe { self.root_va.add(l1_idx(ipa)) };
        let l1e = unsafe { *l1_ptr };
        let l2_va: *mut u64 = if l1e & (DESC_VALID | DESC_TABLE) == (DESC_VALID | DESC_TABLE) {
            phys_to_virt(desc_pa(l1e)) as *mut u64
        } else if l1e & DESC_VALID == 0 {
            let (va, pa) = self.alloc_subtable().ok_or(S2MapError::OutOfMemory)?;
            unsafe { *l1_ptr = table_desc(pa); }
            va
        } else {
            return Err(S2MapError::BlockConflict);
        };
        let l2_ptr: *mut u64 = unsafe { l2_va.add(l2_idx(ipa)) };
        let l2e = unsafe { *l2_ptr };
        let l3_va: *mut u64 = if l2e & (DESC_VALID | DESC_TABLE) == (DESC_VALID | DESC_TABLE) {
            phys_to_virt(desc_pa(l2e)) as *mut u64
        } else if l2e & DESC_VALID == 0 {
            let (va, pa) = self.alloc_subtable().ok_or(S2MapError::OutOfMemory)?;
            unsafe { *l2_ptr = table_desc(pa); }
            va
        } else {
            return Err(S2MapError::BlockConflict);
        };
        let l3_ptr: *mut u64 = unsafe { l3_va.add(l3_idx(ipa)) };
        // SAFETY: writing a Device-nGnRnE page descriptor for MMIO passthrough.
        unsafe { *l3_ptr = page_desc_device(hpa, writable); }
        Ok(())
    }

    fn unmap_single(&mut self, ipa: u64) {
        let l1_ptr: *mut u64 = unsafe { self.root_va.add(l1_idx(ipa)) };
        let l1e = unsafe { *l1_ptr };
        if l1e & (DESC_VALID | DESC_TABLE) != (DESC_VALID | DESC_TABLE) { return; }

        let l2_va: *mut u64 = phys_to_virt(desc_pa(l1e)) as *mut u64;
        let l2_ptr: *mut u64 = unsafe { l2_va.add(l2_idx(ipa)) };
        let l2e = unsafe { *l2_ptr };
        if l2e & (DESC_VALID | DESC_TABLE) != (DESC_VALID | DESC_TABLE) { return; }

        let l3_va: *mut u64 = phys_to_virt(desc_pa(l2e)) as *mut u64;
        let l3_ptr: *mut u64 = unsafe { l3_va.add(l3_idx(ipa)) };
        // Clear the page descriptor.
        unsafe { *l3_ptr = 0; }
    }
}

// ── Drop — free all allocated frames ────────────────────────────────────────

impl Drop for Stage2Table {
    fn drop(&mut self) {
        let mut g = FRAME_ALLOCATOR.lock();
        if let Some(alloc) = g.as_mut() {
            // Free L2/L3 sub-table frames (individually allocated).
            for &pa in &self.sub_frames {
                alloc.deallocate_frame(pa);
            }
            // Free the 2 × root L1 frames (allocated contiguously).
            let root = self.root_pa as usize;
            alloc.deallocate_frame(root);
            alloc.deallocate_frame(root + PAGE_SIZE);
            // Free guest RAM pages if they were carved here.
            if self.guest_ram_pages > 0 {
                let base = self.guest_ram_pa as usize;
                for i in 0..self.guest_ram_pages {
                    alloc.deallocate_frame(base + i * PAGE_SIZE);
                }
            }
        }
    }
}

// ── Kernel probe (test-cfg) ──────────────────────────────────────────────────

/// Smoke-probe that exercises the Stage-2 table builder without running a vCPU.
///
/// Call from kernel init under `#[cfg(feature = "test-hooks")]` or a CI boot flag.
/// On AArch64 the caller can follow up with `at s12e1r` to verify IPA→HPA via
/// PAR_EL1; on non-AArch64 targets the function tests the software walk only.
///
/// # Panics
/// Panics on any unexpected error — this is a development smoke check.
pub fn probe_stage2_table() {
    // Allocate a minimal table.
    let mut tbl = Stage2Table::new().expect("Stage2Table::new failed");

    // Carve 2 pages of guest RAM (trivial, no RT concern).
    let guest_pa = tbl.carve_guest_ram(2).expect("carve_guest_ram failed");

    // Map guest IPA 0x40000000 → carved PA.
    tbl.map(0x40000000, guest_pa, 2, true).expect("Stage2Table::map failed");

    // Verify the L3 descriptor was written correctly.
    let l1_ptr: *mut u64 = unsafe { tbl.root_va.add(l1_idx(0x40000000)) };
    let l1e = unsafe { *l1_ptr };
    assert!(l1e & (DESC_VALID | DESC_TABLE) == (DESC_VALID | DESC_TABLE), "L1 table entry invalid");

    let l2_va: *mut u64 = phys_to_virt(desc_pa(l1e)) as *mut u64;
    let l2e = unsafe { *l2_va.add(l2_idx(0x40000000)) };
    assert!(l2e & (DESC_VALID | DESC_TABLE) == (DESC_VALID | DESC_TABLE), "L2 table entry invalid");

    let l3_va: *mut u64 = phys_to_virt(desc_pa(l2e)) as *mut u64;
    let l3e = unsafe { *l3_va.add(l3_idx(0x40000000)) };
    assert!(l3e & DESC_VALID != 0, "L3 page descriptor not valid");
    assert_eq!(desc_pa(l3e), guest_pa as usize, "L3 PA mismatch");

    // Unmap and verify cleared.
    tbl.unmap(0x40000000, 2);
    let l3e_after = unsafe { *l3_va.add(l3_idx(0x40000000)) };
    assert_eq!(l3e_after, 0, "L3 descriptor not cleared after unmap");

    // Drop exercises the frame-free path (Law 8).
    drop(tbl);
    // If the frame allocator panics on double-free, the probe would have caught it.

    log::info!("[stage2::probe] Stage-2 table smoke-check passed");
}
