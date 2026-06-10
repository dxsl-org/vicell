//! Paging management for ViCell kernel.
//!
//! Delegates to HAL for architecture-specific page table management (SV39/SV32).

use crate::memory::frame::FrameAllocator;
use crate::*;
// Bare-physical arches (riscv32, x86_32, arm32) have no PageTable implementation.
#[cfg(not(any(target_arch = "riscv32", target_arch = "x86", target_arch = "arm")))]
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

// Re-export PageFlags from HAL. PageFlags is in hal-paging (always compiled) so
// it is available on all targets including riscv32.
pub use hal::PageFlags as Flags;

use crate::sync::Spinlock;

/// Global Kernel Root Page Table Address
pub static KERNEL_ROOT: Spinlock<Option<PhysAddr>> = Spinlock::new(None);

/// No-op paging init for bare physical addressing (RV32 Nano, SATP=0).
///
/// Phase-31 uses identity-mapped physical memory with no page tables.
/// Call in place of `init_kernel_paging` + `activate_paging` on riscv32.
pub fn init_bare() {}


/// Initialize the kernel page table (not used on bare-physical arches — call init_bare() instead)
#[cfg(not(any(target_arch = "riscv32", target_arch = "x86", target_arch = "arm")))]
pub fn init_kernel_paging(
    allocator: &mut FrameAllocator,
    mmap: &[crate::boot::MemoryMapEntry],
) -> PagingResult<PhysAddr> {
    // 1. Allocate root page table
    let root_frame = allocator
        .allocate_frame()
        .ok_or(PageTableError::OutOfMemory)?;

    // Zero it out
    unsafe {
        let ptr = root_frame as *mut u8;
        core::ptr::write_bytes(ptr, 0, PAGE_SIZE);
    }

    // Create generic PageTable wrapper at the physical address
    let root_table = unsafe { &mut *(root_frame as *mut PageTable) };

    // 2. Identity map all usable memory and kernel sections
    for entry in mmap.iter() {
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

        let mut alloc_closure = || allocator.allocate_frame();
        root_table
            .identity_map(entry.base, entry.base + entry.length, flags, &mut alloc_closure)
            .map_err(|_| PageTableError::OutOfMemory)?;
    }

    // Explicitly identity-map QEMU virt machine MMIO regions.
    // Limine does NOT include these in its memory map, but the kernel needs
    // them accessible after paging is activated.
    let mmio_flags = PageFlags::from_bits(
        PageFlags::VALID
            | PageFlags::READ
            | PageFlags::WRITE
            | PageFlags::ACCESSED
            | PageFlags::DIRTY,
    );
    let mut alloc_fn = || allocator.allocate_frame();

    #[cfg(target_arch = "riscv64")]
    {
        // MMIO bases from DTB; falls back to QEMU virt defaults when DTB absent.
        // uart_region is aligned to 64 KB to cover UART + adjacent VirtIO MMIO slots.
        let (clint_base, plic_base, plic_size, uart_region) = crate::platform::with(|p| {
            (p.clint_base, p.plic_base, p.plic_size, p.uart_base & !0xFFFF)
        });
        root_table.identity_map(clint_base, clint_base + 0x10000, mmio_flags, &mut alloc_fn)
            .map_err(|_| PageTableError::OutOfMemory)?;
        root_table.identity_map(plic_base, plic_base + plic_size, mmio_flags, &mut alloc_fn)
            .map_err(|_| PageTableError::OutOfMemory)?;
        root_table.identity_map(uart_region, uart_region + 0x10000, mmio_flags, &mut alloc_fn)
            .map_err(|_| PageTableError::OutOfMemory)?;
    }
    #[cfg(target_arch = "aarch64")]
    {
        // QEMU aarch64 virt — map only the peripherals the kernel actually touches.
        // Cell ELFs may load at any VA not covered by these identity maps.
        //
        // GIC (0x0800_0000..0x0900_0000) is intentionally NOT mapped here:
        // the generic timer and external IRQs are disabled for the initial ARM64
        // bring-up, so vi_aarch64_irq_handler is never entered and GIC registers
        // are never accessed.  Omitting this range frees 0x0800_0000 for cell ELFs.
        //
        //   0x0900_0000–0x0900_1000 : PL011 UART (console output)
        //   0x0900_3000–0x0900_4000 : PL061 GPIO (peripheral tests)
        //   0x0A00_0000–0x0A00_4000 : VirtIO ARM64 (32 slots × 0x200)
        //   0x1000_0000–0x1001_0000 : RISC-V VirtIO probe range (virtio_blk/net
        //                             still probe 0x1000_1000 — map to avoid Data Abort)
        root_table.identity_map(0x0900_0000, 0x0900_4000, mmio_flags, &mut alloc_fn)
            .map_err(|_| PageTableError::OutOfMemory)?;
        root_table.identity_map(0x0A00_0000, 0x0A00_4000, mmio_flags, &mut alloc_fn)
            .map_err(|_| PageTableError::OutOfMemory)?;
        root_table.identity_map(0x1000_0000, 0x1001_0000, mmio_flags, &mut alloc_fn)
            .map_err(|_| PageTableError::OutOfMemory)?;
    }
    #[cfg(target_arch = "x86_64")]
    {
        // QEMU q35 MMIO: IOAPIC, HPET, LAPIC.
        // Stored in our PML4 for future activation; Limine already maps these.
        root_table.identity_map(0xFEC0_0000, 0xFEC0_1000, mmio_flags, &mut alloc_fn)
            .map_err(|_| PageTableError::OutOfMemory)?;
        root_table.identity_map(0xFED0_0000, 0xFED0_1000, mmio_flags, &mut alloc_fn)
            .map_err(|_| PageTableError::OutOfMemory)?;
        root_table.identity_map(0xFEE0_0000, 0xFEE0_1000, mmio_flags, &mut alloc_fn)
            .map_err(|_| PageTableError::OutOfMemory)?;
    }

    *KERNEL_ROOT.lock() = Some(root_frame);
    Ok(root_frame)
}

/// Helper to remap a range of memory with USER permissions.
#[cfg(not(any(target_arch = "riscv32", target_arch = "x86", target_arch = "arm")))]
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

/// Activate virtual memory (not used on bare-physical arches).
///
/// # Safety
/// This function enables paging. The root table MUST contain a valid identity mapping.
#[cfg(not(any(target_arch = "riscv32", target_arch = "x86", target_arch = "arm")))]
pub unsafe fn activate_paging(root_table_phys: PhysAddr) {
    let root_table = &*(root_table_phys as *const PageTable);
    root_table.activate();
}

/// Translate a virtual address to its physical address by walking the kernel
/// page table.
///
/// Bare-physical arches (riscv32, x86_32, arm32): VA == PA — use the identity stub below.
/// Paged arches (riscv64, aarch64) read the page-table root from the CSR.
#[cfg(not(any(target_arch = "riscv32", target_arch = "x86", target_arch = "arm")))]
pub fn virt_to_phys(vaddr: VAddr) -> Option<PhysAddr> {
    use hal::traits::PageTableTrait;

    #[cfg(target_arch = "riscv64")]
    {
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
        return root_table.translate(vaddr);
    }
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: TTBR0_EL1 holds the root page table physical address written
        // by activate_paging(). Valid and stable after boot.
        let ttbr0: usize;
        unsafe { core::arch::asm!("mrs {}, ttbr0_el1", out(reg) ttbr0, options(nomem, nostack)) };
        let root_phys = ttbr0 & !0xFFF; // mask off ASID/flags in bits [63:48] and [11:0]
        if root_phys == 0 { return None; }
        let root_table = unsafe { &*(root_phys as *const hal::PageTable) };
        return root_table.translate(vaddr);
    }
    #[allow(unreachable_code)]
    None
}

/// Bare-physical identity translation: physical == virtual.
/// Used by riscv32 (SATP=0), x86_32 (CR0.PG=0), and arm32 (MMU off).
#[cfg(any(target_arch = "riscv32", target_arch = "x86", target_arch = "arm"))]
pub fn virt_to_phys(vaddr: VAddr) -> Option<PhysAddr> {
    Some(vaddr)
}

/// Unmap a virtual page in the kernel address space (clears the PTE).
/// Used to create true guard pages that trap on access.
/// Bare-physical arches: no page tables — returns NotSupported.
#[cfg(not(any(target_arch = "riscv32", target_arch = "x86", target_arch = "arm")))]
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

/// Map a page in the kernel address space.
/// Bare-physical arches (riscv32, x86, arm): no page tables — stub below returns NotSupported.
#[cfg(not(any(target_arch = "riscv32", target_arch = "x86", target_arch = "arm")))]
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

// Bare-physical stubs: no page-table operations for riscv32, x86_32, arm32.
// These arches run with paging disabled; these stubs satisfy the compiler
// without ever being called at runtime.
#[cfg(any(target_arch = "riscv32", target_arch = "x86", target_arch = "arm"))]
pub fn unmap_page(_vaddr: VAddr) -> PagingResult<()> {
    Err(PageTableError::NotSupported)
}

#[cfg(any(target_arch = "riscv32", target_arch = "x86", target_arch = "arm"))]
pub fn map_page(
    _allocator: &mut FrameAllocator,
    _vaddr: VAddr,
    _paddr: PhysAddr,
    _flags: Flags,
) -> PagingResult<()> {
    Err(PageTableError::NotSupported)
}

#[cfg(any(target_arch = "riscv32", target_arch = "x86", target_arch = "arm"))]
pub fn remap_range_user(_start: PhysAddr, _pages: usize) {
    // No-op: paging disabled.
}
