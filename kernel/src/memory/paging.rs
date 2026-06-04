//! Paging management for ViOS kernel.
//!
//! Delegates to HAL for architecture-specific page table management (SV39/SV32).

use crate::memory::frame::FrameAllocator;
use crate::*;
use hal::{PageFlags, PageTable, PageTableTrait};

/// Page size (4KB)
pub const PAGE_SIZE: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageTableError {
    OutOfMemory,
    InvalidAddress,
    NotSupported,
}

pub type PagingResult<T> = core::result::Result<T, PageTableError>;

// Re-export PageFlags from HAL for convenience
pub use hal::PageFlags as Flags;

use crate::sync::Spinlock;

/// Global Kernel Root Page Table Address
pub static KERNEL_ROOT: Spinlock<Option<PhysAddr>> = Spinlock::new(None);

// Helper for debugging
fn puts(s: &str) {
    for c in s.bytes() {
        let _ = crate::hal::sbi::console_putchar(c);
    }
}

/// Initialize the kernel page table
pub fn init_kernel_paging(
    allocator: &mut FrameAllocator,
    mmap: &[crate::boot::MemoryMapEntry],
) -> PagingResult<PhysAddr> {
    puts("TRACE: init_kernel_paging start\n");
    // 1. Allocate root page table
    let root_frame = allocator
        .allocate_frame()
        .ok_or(PageTableError::OutOfMemory)?;
    
    puts("TRACE: root_frame allocated\n");

    // Zero it out
    unsafe {
        let ptr = root_frame as *mut u8;
        core::ptr::write_bytes(ptr, 0, PAGE_SIZE);
    }

    // Create generic PageTable wrapper at the physical address
    let root_table = unsafe { &mut *(root_frame as *mut PageTable) };

    // 2. Identity map all usable memory and kernel sections
    for (_i, entry) in mmap.iter().enumerate() {
        puts("TRACE: mapping region ");
        // We can't format easily, so just trace index
        // puts(entry.ty as u8 + '0' as u8 ...); 
        
        let flags = match entry.ty {
            crate::boot::MemoryType::Usable => PageFlags::from_bits(
                PageFlags::VALID
                    | PageFlags::READ
                    | PageFlags::WRITE
                    | PageFlags::EXECUTE
                    | PageFlags::ACCESSED
                    | PageFlags::DIRTY,
            ),
            crate::boot::MemoryType::Kernel => PageFlags::from_bits(
                PageFlags::VALID
                    | PageFlags::READ
                    | PageFlags::WRITE
                    | PageFlags::EXECUTE
                    | PageFlags::ACCESSED
                    | PageFlags::DIRTY,
            ),
            crate::boot::MemoryType::Bootloader => PageFlags::from_bits(
                PageFlags::VALID
                    | PageFlags::READ
                    | PageFlags::WRITE
                    | PageFlags::ACCESSED
                    | PageFlags::DIRTY,
            ),
            crate::boot::MemoryType::Framebuffer => PageFlags::from_bits(
                PageFlags::VALID
                    | PageFlags::READ
                    | PageFlags::WRITE
                    | PageFlags::ACCESSED
                    | PageFlags::DIRTY,
            ),
            crate::boot::MemoryType::MMIO => PageFlags::from_bits(
                PageFlags::VALID
                    | PageFlags::READ
                    | PageFlags::WRITE
                    | PageFlags::ACCESSED
                    | PageFlags::DIRTY,
            ),
            _ => continue,
        };

        puts("TRACE: mapping... \n");
        // Identity map this region using HAL
        let mut alloc_closure = || allocator.allocate_frame();

        root_table
            .identity_map(
                entry.base,
                entry.base + entry.length,
                flags,
                &mut alloc_closure,
            )
            .map_err(|_| PageTableError::OutOfMemory)?;
        
        puts("TRACE: mapped region\n");
    }

    // Explicitly identity-map QEMU virt machine MMIO regions.
    // Limine does NOT include these in its memory map, but the kernel needs
    // them accessible after paging is activated (UART, VirtIO block/input/GPU/net).
    //
    // QEMU virt memory map (hard-coded for riscv64 virt machine):
    //   0x0000_0000 - 0x0000_0FFF : internal ROM / debug
    //   0x0200_0000 - 0x0200_FFFF : CLINT
    //   0x0C00_0000 - 0x0FFF_FFFF : PLIC
    //   0x1000_0000 - 0x1000_0FFF : UART 16550A
    //   0x1000_1000 - 0x1000_8FFF : VirtIO MMIO (8 devices)
    let mmio_flags = PageFlags::from_bits(
        PageFlags::VALID
            | PageFlags::READ
            | PageFlags::WRITE
            | PageFlags::ACCESSED
            | PageFlags::DIRTY,
    );
    let mut alloc_fn = || allocator.allocate_frame();

    // CLINT
    root_table.identity_map(0x0200_0000, 0x0201_0000, mmio_flags, &mut alloc_fn)
        .map_err(|_| PageTableError::OutOfMemory)?;
    // PLIC (16MB range)
    root_table.identity_map(0x0C00_0000, 0x1000_0000, mmio_flags, &mut alloc_fn)
        .map_err(|_| PageTableError::OutOfMemory)?;
    // UART + VirtIO MMIO (64KB covers all 8 VirtIO slots)
    root_table.identity_map(0x1000_0000, 0x1001_0000, mmio_flags, &mut alloc_fn)
        .map_err(|_| PageTableError::OutOfMemory)?;

    puts("TRACE: MMIO regions mapped\n");

    // Store globally
    *KERNEL_ROOT.lock() = Some(root_frame);

    puts("TRACE: init_kernel_paging success\n");

    Ok(root_frame)
}

/// Helper to remap a range of memory with USER permissions.
/// Used for User Stacks which are allocated in Identity Map (Usable RAM).
pub fn remap_range_user(start: PhysAddr, pages: usize) {
    let mut root_guard = KERNEL_ROOT.lock();
    if let Some(root_addr) = root_guard.as_mut() {
        // We cast the physical address directly to the PageTable struct reference
        // This is valid in Identity Map which kernel uses
        let table = unsafe { &mut *(*root_addr as *mut hal::paging::PageTable) };

        let mut frame_guard = crate::memory::frame::FRAME_ALLOCATOR.lock();
        if let Some(allocator) = frame_guard.as_mut() {
            let mut alloc_closure = || allocator.allocate_frame();

            use hal::traits::PageTableTrait;
            // Add USER flag to allow U-mode access
            let flags = PageFlags::from_bits(
                PageFlags::VALID
                    | PageFlags::READ
                    | PageFlags::WRITE
                    | PageFlags::EXECUTE
                    | PageFlags::USER
                    | PageFlags::ACCESSED
                    | PageFlags::DIRTY,
            );

            use hal::paging::PAGE_SIZE;
            for i in 0..pages {
                let addr = start + (i * PAGE_SIZE);
                // Identity map: Virt = Phys
                // We overwrite existing mapping with new flags
                let _ = table
                    .map(addr, addr, flags, &mut alloc_closure)
                    .expect("Failed to map user stack page!");
            }
        }
    }
}

/// Activate virtual memory
///
/// # Safety
/// This function enables paging. The root table MUST contain a valid identity mapping.
pub unsafe fn activate_paging(root_table_phys: PhysAddr) {
    let root_table = &*(root_table_phys as *const PageTable);
    root_table.activate();
}

/// Translate a virtual address to its physical address by walking the kernel
/// page table.
///
/// Returns `Some(phys)` when a valid leaf PTE is found, `None` when the
/// virtual address is unmapped.  Used by `VirtioHal::share()` so DMA
/// descriptors carry the correct physical address regardless of whether the
/// buffer lives in identity-mapped memory (kernel stack) or a remapped ELF
/// segment (e.g. VFS cell BSS / static sector buffers).
/// Translate a virtual address to its physical address by walking the kernel
/// page table.
///
/// Reads the page-table root from the `satp` CSR directly (no lock taken) so
/// this function is safe to call from any context, including interrupt handlers
/// or while holding other locks (e.g. the VirtIO `BLOCK_DEVICE` lock that is
/// held during `read_blocks`/`write_blocks`).  The page table root never
/// changes after boot so reading satp without the KERNEL_ROOT lock is correct.
pub fn virt_to_phys(vaddr: VAddr) -> Option<PhysAddr> {
    use hal::traits::PageTableTrait;
    // SAFETY: `satp` CSR holds the page-table root PPN (bits 43:0 in Sv39 mode).
    // We only read it here; no write, so no ordering concern.
    let satp: usize;
    unsafe { core::arch::asm!("csrr {}, satp", out(reg) satp) };
    // Sv39 PPN field is bits 43:0 of satp; shift left by 12 to get PA.
    let root_ppn = satp & ((1 << 44) - 1);
    let root_phys = root_ppn << 12;
    if root_phys == 0 { return None; }
    // SAFETY: root_phys is the physical address of the active root PageTable;
    // valid and stable for the kernel's lifetime after `init_kernel_paging`.
    let root_table = unsafe { &*(root_phys as *const hal::PageTable) };
    root_table.translate(vaddr)
}

/// Unmap a virtual page in the kernel address space (clears the PTE).
/// Used to create true guard pages that trap on access.
pub fn unmap_page(vaddr: VAddr) -> PagingResult<()> {
    let root_lock = KERNEL_ROOT.lock();
    if let Some(root_phys) = *root_lock {
        let root_table = unsafe { &mut *(root_phys as *mut PageTable) };
        root_table.unmap(vaddr).map_err(|_| PageTableError::InvalidAddress)?;
        Ok(())
    } else {
        Err(PageTableError::NotSupported)
    }
}

/// Map a page in the kernel address space
pub fn map_page(
    allocator: &mut FrameAllocator,
    vaddr: VAddr,
    paddr: PhysAddr,
    flags: Flags,
) -> PagingResult<()> {
    let root_lock = KERNEL_ROOT.lock();
    if let Some(root_phys) = *root_lock {
        let root_table = unsafe { &mut *(root_phys as *mut PageTable) };
        // Allocator is passed in, so we don't lock here.

        let mut alloc_closure = || allocator.allocate_frame();

        root_table
            .map(vaddr, paddr, flags, &mut alloc_closure)
            .map_err(|_| PageTableError::OutOfMemory)?;
        Ok(())
    } else {
        Err(PageTableError::NotSupported) // Paging not initialized
    }
}
