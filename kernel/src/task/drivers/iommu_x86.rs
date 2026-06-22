//! Intel VT-d DMA isolation driver for x86_64.
//!
//! Phase 1 `init_hw()`:   probe VT-d (GCAP probe), allocate root/context/SLPT tables.
//!                         Does NOT enable translation — VT-d stays silent.
//! Phase 2 `map_range()`: add a physical range to the VT-d SLPT.
//! Phase 3 `activate()`:  fill context entries with TT=TRANSLATED+SLPTPTR, enable TE.
//!
//! After activation, DMA is restricted to SLPT-mapped ranges. Any other IOVA
//! triggers a VT-d fault (TT=0b00 with no matching leaf entry).

use alloc::alloc::{alloc_zeroed, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};
use crate::sync::Spinlock;
use super::iommu_pt::VtdSlpt;

// VT-d MMIO register offsets (Intel VT-d spec §10.4)
const VTD_GCAP:   usize = 0x00; // 64-bit capabilities (read-only)
const VTD_GCMD:   usize = 0x18; // 32-bit global command (write-only)
const VTD_GSTS:   usize = 0x1C; // 32-bit global status (read-only)
const VTD_RTADDR: usize = 0x20; // 64-bit root table address

// GCMD / GSTS bit masks
const TE:   u32 = 1 << 31; // Translation Enable
const SRTP: u32 = 1 << 30; // Set Root Table Pointer / Root Table Pointer Set

// Context entry encoding (Intel VT-d spec §9.3, lo qword)
//   Bits [3:2] = TT: 0b00 = DMA remapping via SLPT (default, no explicit constant needed)
//   Bits [6:4] = AW: 0b010 = 39-bit (3-level SLPT)
//   Bit  [0]   = P:  Present
const AW_39BIT:    u64 = 0b010 << 4; // Address Width = 39-bit → 3-level SLPT
const CTX_PRESENT: u64 = 1;
const DID:         u64 = 0x0001u64 << 8; // Domain ID placed in hi qword bits[23:8]

// QEMU q35 hardcoded VT-d MMIO base (must be identity-mapped by init_kernel_paging_x86).
const VTD_BASE:  usize = 0xFED9_0000;
const POLL_MAX:  u64   = 1_000_000;

// ── Module-level state ────────────────────────────────────────────────────────

static VTD_ROOT_VIRT: AtomicUsize = AtomicUsize::new(0);
static VTD_ROOT_PHYS: AtomicUsize = AtomicUsize::new(0);
static VTD_CTX_VIRT:  AtomicUsize = AtomicUsize::new(0);
static VTD_SLPT: Spinlock<Option<VtdSlpt>> = Spinlock::new(None);

// ── MMIO helpers ─────────────────────────────────────────────────────────────

#[inline]
unsafe fn read64(base: usize, off: usize) -> u64 {
    // SAFETY: caller ensures base is identity-mapped VT-d MMIO.
    unsafe { core::ptr::read_volatile((base + off) as *const u64) }
}
#[inline]
unsafe fn read32(base: usize, off: usize) -> u32 {
    // SAFETY: caller ensures base is identity-mapped VT-d MMIO.
    unsafe { core::ptr::read_volatile((base + off) as *const u32) }
}
#[inline]
unsafe fn write32(base: usize, off: usize, val: u32) {
    // SAFETY: caller ensures base is identity-mapped VT-d MMIO.
    unsafe { core::ptr::write_volatile((base + off) as *mut u32, val) }
}
#[inline]
unsafe fn write64(base: usize, off: usize, val: u64) {
    // SAFETY: caller ensures base is identity-mapped VT-d MMIO.
    unsafe { core::ptr::write_volatile((base + off) as *mut u64, val) }
}

/// Convert a kernel heap virtual address to its physical address (x86_64 HHDM).
#[inline]
fn heap_to_phys(virt: usize) -> u64 {
    (virt - crate::memory::frame::phys_to_virt(0)) as u64
}

/// Allocate a zeroed 4 KiB page for an IOMMU table. Panics on OOM.
fn alloc_table() -> (usize, u64) {
    let layout = Layout::from_size_align(4096, 4096).expect("VT-d table layout");
    // SAFETY: layout is non-zero and 4096-aligned.
    let ptr = unsafe { alloc_zeroed(layout) } as usize;
    assert!(ptr != 0, "[vtd] OOM allocating IOMMU table");
    (ptr, heap_to_phys(ptr))
}

// ── Phase 1: probe + allocate ─────────────────────────────────────────────────

/// Probe Intel VT-d and allocate root table, context table, and SLPT.
/// Does NOT enable VT-d translation — stays silent until `activate()`.
pub(super) fn init_hw() {
    // SAFETY: VTD_BASE (0xFED90000) is identity-mapped by init_kernel_paging_x86.
    let gcap = unsafe { read64(VTD_BASE, VTD_GCAP) };
    if gcap == 0 || gcap == u64::MAX {
        log::info!("[vtd] Intel VT-d not present (GCAP={:#x})", gcap);
        return;
    }
    log::info!("[vtd] Intel VT-d found GCAP={:#x}", gcap);

    let (root_virt, root_phys) = alloc_table();
    let (ctx_virt, _ctx_phys)  = alloc_table();
    let slpt = VtdSlpt::new();

    VTD_ROOT_VIRT.store(root_virt,       Ordering::Relaxed);
    VTD_ROOT_PHYS.store(root_phys as usize, Ordering::Relaxed);
    VTD_CTX_VIRT.store(ctx_virt,         Ordering::Relaxed);
    *VTD_SLPT.lock() = Some(slpt);

    log::info!("[vtd] VT-d structures allocated — DMA isolation pending activation");
}

// ── Phase 2: register DMA range ───────────────────────────────────────────────

/// Add [phys, phys+size) to the VT-d SLPT.
pub(super) fn map_range(phys: u64, size: usize) {
    if let Some(slpt) = VTD_SLPT.lock().as_ref() {
        slpt.map_range(phys, size);
    }
}

// ── Phase 3: activate enforcement ────────────────────────────────────────────

/// Fill VT-d context entries with TT=TRANSLATED+SLPT, then enable translation.
///
/// Context entries use TT=0b00 (DMA remapping) which requires a valid SLPT.
/// Any IOVA not mapped in the SLPT triggers a VT-d fault.
pub(super) fn activate() {
    let root_virt = VTD_ROOT_VIRT.load(Ordering::Relaxed);
    let root_phys = VTD_ROOT_PHYS.load(Ordering::Relaxed) as u64;
    let ctx_virt  = VTD_CTX_VIRT.load(Ordering::Relaxed);
    if root_virt == 0 { return; } // VT-d not present

    let slpt_root = match VTD_SLPT.lock().as_ref() {
        Some(s) => s.root_phys(),
        None => return,
    };

    // Context entry lo qword: TT=0b00 (translated), AW=39-bit, SLPTPTR, Present.
    // TT=0b00 is the default (bits 3:2 = 0), so only AW+Present+SLPTPTR are set.
    let lo = (slpt_root & !0xFFF) | AW_39BIT | CTX_PRESENT;
    let hi = DID;
    for i in 0usize..256 {
        let slot = ctx_virt + i * 16;
        // SAFETY: ctx_virt is a zeroed 4096-B page; i*16 < 4096.
        unsafe {
            core::ptr::write_volatile(slot       as *mut u64, lo);
            core::ptr::write_volatile((slot + 8) as *mut u64, hi);
        }
    }

    // Root table: all 256 bus entries point to the shared context table.
    let ctx_phys = heap_to_phys(ctx_virt);
    for i in 0usize..256 {
        let slot = root_virt + i * 16;
        // SAFETY: root_virt is a zeroed 4096-B page; i*16 < 4096.
        unsafe {
            core::ptr::write_volatile(slot       as *mut u64, ctx_phys | CTX_PRESENT);
            core::ptr::write_volatile((slot + 8) as *mut u64, 0u64);
        }
    }

    // Step 1: programme root table address.
    // SAFETY: VTD_BASE is identity-mapped; root_phys is 4096-aligned.
    unsafe { write64(VTD_BASE, VTD_RTADDR, root_phys); }

    // Step 2: GCMD.SRTP → poll GSTS.RTPS.
    unsafe { write32(VTD_BASE, VTD_GCMD, SRTP); }
    let mut n = 0u64;
    loop {
        if unsafe { read32(VTD_BASE, VTD_GSTS) } & SRTP != 0 { break; }
        n += 1;
        if n >= POLL_MAX { log::warn!("[vtd] GSTS.RTPS never set — aborting"); return; }
        core::hint::spin_loop();
    }

    // Step 3: GCMD.(TE|SRTP) → poll GSTS.TES.
    unsafe { write32(VTD_BASE, VTD_GCMD, TE | SRTP); }
    let mut n = 0u64;
    loop {
        if unsafe { read32(VTD_BASE, VTD_GSTS) } & TE != 0 { break; }
        n += 1;
        if n >= POLL_MAX { log::warn!("[vtd] GSTS.TES never set — translation NOT active"); return; }
        core::hint::spin_loop();
    }

    super::iommu::set_active();
    log::info!("[vtd] Intel VT-d: DMA isolation ACTIVE (TT=TRANSLATED, Sv39 SLPT @ {:#x})",
               slpt_root);
}
