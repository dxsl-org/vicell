//! x86_64 4-level page table (PML4->PDPT->PD->PT), 4KB and 2MB pages.
//!
//! Traversal assumption: PTE entries store physical addresses which are
//! directly dereferenceable as virtual addresses (phys == virt) while the
//! Limine identity map is active.  Once the kernel activates its own PML4
//! via `activate()`, subsequent calls to `map`/`translate`/`unmap` must
//! apply the HHDM offset before dereferencing PTE physical addresses.
//! This is deferred to the full VFS/memory phase.
use hal_paging::{PageFlags, PageTableTrait};
use types::*;
use core::arch::asm;

pub const PAGE_SIZE: usize = 4096;

const PTE_P:  u64 = 1<<0;
const PTE_RW: u64 = 1<<1;
const PTE_US: u64 = 1<<2;
const PTE_PS: u64 = 1<<7;
const PTE_NX: u64 = 1<<63;

#[repr(C, align(4096))]
pub struct PageTable { entries: [u64; 512] }
impl PageTable { pub const fn zero() -> Self { Self { entries: [0u64; 512] } } }

impl PageTableTrait for PageTable {
    fn init(&mut self) -> ViResult<PhysAddr> {
        self.entries = [0u64; 512];
        Ok(self as *mut _ as PhysAddr)
    }
    fn map(&mut self, virt: VAddr, phys: PhysAddr, flags: PageFlags,
           alloc_fn: &mut dyn FnMut() -> Option<PhysAddr>) -> ViResult<()> {
        let i3 = (virt>>39)&0x1FF; let i2=(virt>>30)&0x1FF;
        let i1 = (virt>>21)&0x1FF; let i0=(virt>>12)&0x1FF;
        let pdpt = self.get_or_alloc(i3, alloc_fn)?;
        let pd   = pdpt.get_or_alloc(i2, alloc_fn)?;
        let pt   = pd.get_or_alloc(i1, alloc_fn)?;
        let mut e = phys as u64 | PTE_P;
        if flags.bits()&PageFlags::WRITE   !=0 { e|=PTE_RW; }
        if flags.bits()&PageFlags::USER    !=0 { e|=PTE_US; }
        if flags.bits()&PageFlags::EXECUTE ==0 { e|=PTE_NX; }
        pt.entries[i0] = e;
        Ok(())
    }
    fn unmap(&mut self, virt: VAddr) -> ViResult<()> {
        let e0=self.entries[(virt>>39)&0x1FF];
        if e0&PTE_P==0 { return Err(ViError::NotFound); }
        let pdpt: &mut PageTable = unsafe { &mut *((e0&!0xFFF) as *mut PageTable) };
        let e1=pdpt.entries[(virt>>30)&0x1FF];
        if e1&PTE_P==0 { return Err(ViError::NotFound); }
        let pd: &mut PageTable = unsafe { &mut *((e1&!0xFFF) as *mut PageTable) };
        let e2=pd.entries[(virt>>21)&0x1FF];
        if e2&PTE_P==0 { return Err(ViError::NotFound); }
        let pt: &mut PageTable = unsafe { &mut *((e2&!0xFFF) as *mut PageTable) };
        pt.entries[(virt>>12)&0x1FF] = 0;
        // SAFETY: invlpg flushes only the one virtual address from the TLB.
        unsafe { asm!("invlpg [{v}]", v=in(reg) virt, options(nomem)); }
        Ok(())
    }
    fn translate(&self, virt: VAddr) -> Option<PhysAddr> {
        let e0=self.entries[(virt>>39)&0x1FF]; if e0&PTE_P==0 {return None;}
        let pdpt: &PageTable = unsafe { &*((e0&!0xFFF) as *const PageTable) };
        let e1=pdpt.entries[(virt>>30)&0x1FF]; if e1&PTE_P==0 {return None;}
        let pd: &PageTable = unsafe { &*((e1&!0xFFF) as *const PageTable) };
        let e2=pd.entries[(virt>>21)&0x1FF]; if e2&PTE_P==0 {return None;}
        if e2&PTE_PS!=0 { return Some(((e2&!0x1F_FFFF)+(virt&0x1F_FFFF) as u64) as PhysAddr); }
        let pt: &PageTable = unsafe { &*((e2&!0xFFF) as *const PageTable) };
        let e3=pt.entries[(virt>>12)&0x1FF]; if e3&PTE_P==0 {return None;}
        Some(((e3&!0xFFF)+(virt&0xFFF) as u64) as PhysAddr)
    }
    unsafe fn activate(&self) {
        let cr3 = self as *const _ as u64;
        // SAFETY: CR3 write activates new PML4; caller ensures identity mapping covers
        // the instruction pointer so execution continues after the write.
        unsafe { asm!("mov cr3, {}", in(reg) cr3, options(nomem, nostack)); }
    }
}

impl PageTable {
    fn get_or_alloc(&mut self, idx: usize, alloc_fn: &mut dyn FnMut()->Option<PhysAddr>)
        -> ViResult<&mut PageTable> {
        if self.entries[idx]&PTE_P==0 {
            let f = alloc_fn().ok_or(ViError::OutOfMemory)?;
            // SAFETY: f is a freshly-allocated 4KB frame; identity-mapped pre-paging.
            unsafe { core::ptr::write_bytes(f as *mut u8, 0, PAGE_SIZE) };
            self.entries[idx] = f as u64 | PTE_P | PTE_RW;
        }
        let next = (self.entries[idx]&!0xFFF) as PhysAddr;
        // SAFETY: identity-mapped; next is a valid page table frame.
        Ok(unsafe { &mut *(next as *mut PageTable) })
    }
}
