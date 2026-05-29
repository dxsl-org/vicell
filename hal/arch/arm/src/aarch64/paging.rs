//! AArch64 3-level (L1-L3) page table with 4KB granule.
//!
//! Uses TTBR0_EL1 for identity-mapped kernel + MMIO.  TCR_EL1 configured
//! for a 39-bit VA space (T0SZ=25) matching QEMU virt physical range.

use hal_paging::{PageFlags, PageTableTrait};
use types::*;

pub const PAGE_SIZE: usize = 4096;

const PTE_VALID:  u64 = 1 << 0;
const PTE_TABLE:  u64 = 1 << 1;
const PTE_PAGE:   u64 = 1 << 1;
const PTE_AF:     u64 = 1 << 10;
const PTE_SH_IS:  u64 = 3 << 8;
const PTE_AP_EL0: u64 = 1 << 6;
const PTE_UXN:    u64 = 1 << 54;
const PTE_PXN:    u64 = 1 << 53;
const ATTR_NORMAL: u64 = 1 << 2;

fn phys_to_pte_addr(phys: PhysAddr) -> u64 {
    ((phys as u64) >> 12) << 12
}

#[repr(C, align(4096))]
pub struct PageTable {
    entries: [u64; 512],
}

impl PageTable {
    pub const fn zero() -> Self { Self { entries: [0u64; 512] } }
}

impl PageTableTrait for PageTable {
    fn init(&mut self) -> ViResult<PhysAddr> {
        self.entries = [0u64; 512];
        Ok(self as *mut _ as PhysAddr)
    }

    fn map(
        &mut self,
        virt: VAddr,
        phys: PhysAddr,
        flags: PageFlags,
        alloc_fn: &mut dyn FnMut() -> Option<PhysAddr>,
    ) -> ViResult<()> {
        let l1_idx = (virt >> 30) & 0x1FF;
        let l2_idx = (virt >> 21) & 0x1FF;
        let l3_idx = (virt >> 12) & 0x1FF;

        let l2_table = self.get_or_alloc(l1_idx, alloc_fn)?;
        let l3_table = l2_table.get_or_alloc(l2_idx, alloc_fn)?;

        let mut entry = phys_to_pte_addr(phys) | PTE_VALID | PTE_PAGE | PTE_AF | PTE_SH_IS | ATTR_NORMAL;

        if flags.bits() & PageFlags::USER != 0 {
            entry |= PTE_AP_EL0 | PTE_PXN;
        } else {
            entry |= PTE_UXN;
        }
        if flags.bits() & PageFlags::EXECUTE == 0 {
            entry |= PTE_UXN | PTE_PXN;
        }

        l3_table.entries[l3_idx] = entry;
        Ok(())
    }

    fn unmap(&mut self, virt: VAddr) -> ViResult<()> {
        let l1_idx = (virt >> 30) & 0x1FF;
        let l2_idx = (virt >> 21) & 0x1FF;
        let l3_idx = (virt >> 12) & 0x1FF;

        let l1_entry = self.entries[l1_idx];
        if l1_entry & PTE_VALID == 0 { return Err(ViError::NotFound); }
        let l2: &mut PageTable = unsafe { &mut *((l1_entry & !0xFFF) as *mut PageTable) };
        let l2_entry = l2.entries[l2_idx];
        if l2_entry & PTE_VALID == 0 { return Err(ViError::NotFound); }
        let l3: &mut PageTable = unsafe { &mut *((l2_entry & !0xFFF) as *mut PageTable) };
        l3.entries[l3_idx] = 0;
        unsafe { core::arch::asm!("tlbi vaae1is, {}", in(reg) (virt >> 12) as u64, options(nomem)); }
        unsafe { core::arch::asm!("dsb sy", options(nomem, nostack)); }
        Ok(())
    }

    fn translate(&self, virt: VAddr) -> Option<PhysAddr> {
        let l1_idx = (virt >> 30) & 0x1FF;
        let l2_idx = (virt >> 21) & 0x1FF;
        let l3_idx = (virt >> 12) & 0x1FF;
        let l1_entry = self.entries[l1_idx];
        if l1_entry & PTE_VALID == 0 { return None; }
        let l2: &PageTable = unsafe { &*((l1_entry & !0xFFF) as *const PageTable) };
        let l2_entry = l2.entries[l2_idx];
        if l2_entry & PTE_VALID == 0 { return None; }
        let l3: &PageTable = unsafe { &*((l2_entry & !0xFFF) as *const PageTable) };
        let l3_entry = l3.entries[l3_idx];
        if l3_entry & PTE_VALID == 0 { return None; }
        Some(((l3_entry & !0xFFF) as PhysAddr) | (virt & 0xFFF))
    }

    unsafe fn activate(&self) {
        let ttbr0 = self as *const _ as u64;
        let mair: u64 = 0x00FF_0000_0000_0000; // index1=Normal; index0=Device-nGnRnE
        let tcr: u64 = 25      // T0SZ=25 (39-bit VA)
                     | (1 << 8)  // IRGN0=WB-WA
                     | (1 << 10) // ORGN0=WB-WA
                     | (3 << 12) // SH0=Inner-shareable
                     | (0 << 14) // TG0=4KB
                     | (1 << 23); // EPD1=disable TTBR1
        // SAFETY: MMU activation sequence per AArch64 Architecture Reference Manual.
        // Order: write MAIR/TCR, then TTBR0, then barriers, then enable in SCTLR.
        unsafe {
            core::arch::asm!(
                "msr mair_el1, {mair}",
                "msr tcr_el1,  {tcr}",
                "isb",
                "msr ttbr0_el1, {ttbr0}",
                "dsb sy",
                "isb",
                // Invalidate all TLB entries before enabling the MMU so stale entries
            // do not cause faults on real hardware (per ARM ARM DDI 0487 D13.2.118).
            "tlbi vmalle1",   // invalidate all EL1 TLB entries
            "dsb nsh",        // ensure invalidation visible across inner-shareable domain
            "isb",
            "mrs x9, sctlr_el1",
                "orr x9, x9, #(1 << 0)",
                "orr x9, x9, #(1 << 2)",
                "orr x9, x9, #(1 << 12)",
                "msr sctlr_el1, x9",
                "dsb sy",
                "isb",
                mair  = in(reg) mair,
                tcr   = in(reg) tcr,
                ttbr0 = in(reg) ttbr0,
                out("x9") _,
                options(nostack),
            );
        }
    }
}

impl PageTable {
    fn get_or_alloc(
        &mut self,
        idx: usize,
        alloc_fn: &mut dyn FnMut() -> Option<PhysAddr>,
    ) -> ViResult<&mut PageTable> {
        let entry = self.entries[idx];
        if entry & PTE_VALID == 0 {
            let frame = alloc_fn().ok_or(ViError::OutOfMemory)?;
            // SAFETY: frame is a freshly allocated 4KB frame; identity-mapped pre-MMU.
            unsafe { core::ptr::write_bytes(frame as *mut u8, 0, PAGE_SIZE) };
            self.entries[idx] = (frame as u64) | PTE_VALID | PTE_TABLE;
        }
        let next_phys = (self.entries[idx] & !0xFFF) as PhysAddr;
        // SAFETY: identity-mapped; next_phys is a valid page table frame.
        Ok(unsafe { &mut *(next_phys as *mut PageTable) })
    }
}
