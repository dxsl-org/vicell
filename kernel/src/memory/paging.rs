//! Paging management for ViCell kernel.
//!
//! Architecture dispatch:
//!   riscv64, aarch64   — SV39 / AArch64 page tables via HAL `PageTable`
//!   x86_64             — 4-level PML4, dedicated path (own walker + HHDM-aware)
//!   riscv32, x86, arm  — bare physical (no page tables); stubs return NotSupported

use crate::memory::frame::FrameAllocator;
use crate::*;

// HAL PageTable / PageFlags used by riscv64 + aarch64.
#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
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

/// Global Kernel Root Page Table Address (physical).
pub static KERNEL_ROOT: Spinlock<Option<PhysAddr>> = Spinlock::new(None);

/// No-op paging init for bare physical addressing (RV32 Nano, SATP=0).
///
/// Phase-31 uses identity-mapped physical memory with no page tables.
/// Call in place of `init_kernel_paging` + `activate_paging` on riscv32.
pub fn init_bare() {}

/// Flush all TLB entries. Call after unmapping PTEs to prevent stale entries.
///
/// RISC-V:   sfence.vma — full TLB shootdown from S-mode.
/// AArch64:  tlbi vmalle1is + dsb sy + isb — broadcast flush, all ASIDs, EL1.
/// x86_64:   write CR3 to itself (reloads PML4 and flushes all non-global TLB entries).
/// Bare physical (riscv32/x86/arm): no-op.
#[inline(always)]
pub fn tlb_flush_all() {
    #[cfg(target_arch = "riscv64")]
    // SAFETY: sfence.vma is a privileged S-mode fence; always safe from S-mode.
    unsafe { core::arch::asm!("sfence.vma", options(nostack)); }

    #[cfg(target_arch = "aarch64")]
    // SAFETY: tlbi vmalle1is invalidates all EL1 TLB entries broadcast across CPUs.
    // dsb sy ensures the invalidation completes before subsequent memory accesses.
    // isb serializes the instruction stream so the next fetch sees the clean TLB.
    unsafe { core::arch::asm!("tlbi vmalle1is", "dsb sy", "isb", options(nostack, nomem)); }

    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: writing CR3 with its own value flushes all non-global TLB entries.
        // This is a standard x86_64 full-TLB flush idiom; safe from Ring 0.
        unsafe {
            let cr3 = hal::paging::read_cr3();
            hal::paging::write_cr3(cr3);
        }
    }
}

// ─── riscv64 + aarch64 paging ────────────────────────────────────────────────

/// Initialize the kernel page table for riscv64 and aarch64.
///
/// Not used on x86_64 (separate path below) or bare-physical arches.
#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
pub fn init_kernel_paging(
    allocator: &mut FrameAllocator,
    mmap: &[crate::boot::MemoryMapEntry],
) -> PagingResult<PhysAddr> {
    // 1. Allocate root page table frame and zero it.
    let root_frame = allocator
        .allocate_frame()
        .ok_or(PageTableError::OutOfMemory)?;

    // SAFETY: root_frame is freshly allocated; we own it exclusively.
    unsafe {
        core::ptr::write_bytes(root_frame as *mut u8, 0, PAGE_SIZE);
    }

    let root_table = unsafe { &mut *(root_frame as *mut PageTable) };

    // 2. Identity-map all usable memory and kernel sections.
    for entry in mmap.iter() {
        let flags = match entry.ty {
            crate::boot::MemoryType::Usable => PageFlags::from_bits(
                PageFlags::VALID | PageFlags::READ | PageFlags::WRITE
                    | PageFlags::EXECUTE | PageFlags::ACCESSED | PageFlags::DIRTY,
            ),
            crate::boot::MemoryType::Kernel => PageFlags::from_bits(
                PageFlags::VALID | PageFlags::READ | PageFlags::WRITE
                    | PageFlags::EXECUTE | PageFlags::ACCESSED | PageFlags::DIRTY,
            ),
            crate::boot::MemoryType::Bootloader => PageFlags::from_bits(
                PageFlags::VALID | PageFlags::READ | PageFlags::WRITE
                    | PageFlags::ACCESSED | PageFlags::DIRTY,
            ),
            crate::boot::MemoryType::Framebuffer => PageFlags::from_bits(
                PageFlags::VALID | PageFlags::READ | PageFlags::WRITE
                    | PageFlags::ACCESSED | PageFlags::DIRTY,
            ),
            crate::boot::MemoryType::MMIO => PageFlags::from_bits(
                PageFlags::VALID | PageFlags::READ | PageFlags::WRITE
                    | PageFlags::ACCESSED | PageFlags::DIRTY,
            ),
            _ => continue,
        };

        let mut alloc_closure = || allocator.allocate_frame();
        root_table
            .identity_map(entry.base, entry.base + entry.length, flags, &mut alloc_closure)
            .map_err(|_| PageTableError::OutOfMemory)?;
    }

    // 3. Explicitly identity-map arch MMIO regions not in the memory map.
    let mmio_flags = PageFlags::from_bits(
        PageFlags::VALID | PageFlags::READ | PageFlags::WRITE
            | PageFlags::ACCESSED | PageFlags::DIRTY,
    );
    let mut alloc_fn = || allocator.allocate_frame();

    #[cfg(target_arch = "riscv64")]
    {
        let (clint_base, plic_base, plic_size, uart_region) = crate::platform::with(|p| {
            (p.clint_base, p.plic_base, p.plic_size, p.uart_base & !0xFFFF)
        });
        root_table.identity_map(clint_base, clint_base + 0x10000, mmio_flags, &mut alloc_fn)
            .map_err(|_| PageTableError::OutOfMemory)?;
        root_table.identity_map(plic_base, plic_base + plic_size, mmio_flags, &mut alloc_fn)
            .map_err(|_| PageTableError::OutOfMemory)?;
        root_table.identity_map(uart_region, uart_region + 0x10000, mmio_flags, &mut alloc_fn)
            .map_err(|_| PageTableError::OutOfMemory)?;
        // PCIe ECAM bus-0 window (1 MiB at 0x3000_0000) for RISC-V virt gpex.
        // Required before pcie_ecam::init() accesses config space.
        // Only bus 0 is mapped; extend if a PCIe device lands on bus > 0.
        root_table.identity_map(
            crate::task::drivers::pcie_ecam::ECAM_BASE_RISCV,
            crate::task::drivers::pcie_ecam::ECAM_BASE_RISCV
                + crate::task::drivers::pcie_ecam::ECAM_BUS0_SIZE,
            mmio_flags, &mut alloc_fn,
        ).map_err(|_| PageTableError::OutOfMemory)?;
    }
    #[cfg(target_arch = "aarch64")]
    {
        root_table.identity_map(0x0800_0000, 0x0900_0000, mmio_flags, &mut alloc_fn)
            .map_err(|_| PageTableError::OutOfMemory)?;
        root_table.identity_map(0x0900_0000, 0x0904_0000, mmio_flags, &mut alloc_fn)
            .map_err(|_| PageTableError::OutOfMemory)?;
        root_table.identity_map(0x0A00_0000, 0x0A00_4000, mmio_flags, &mut alloc_fn)
            .map_err(|_| PageTableError::OutOfMemory)?;
        root_table.identity_map(0x1000_0000, 0x1001_0000, mmio_flags, &mut alloc_fn)
            .map_err(|_| PageTableError::OutOfMemory)?;
        // PCIe ECAM bus-0 window (1 MiB at 0x3F00_0000) for ARM64 virt gpex.
        // Required before pcie_ecam::init() accesses config space.
        root_table.identity_map(
            crate::task::drivers::pcie_ecam::ECAM_BASE_AARCH64,
            crate::task::drivers::pcie_ecam::ECAM_BASE_AARCH64
                + crate::task::drivers::pcie_ecam::ECAM_BUS0_SIZE,
            mmio_flags, &mut alloc_fn,
        ).map_err(|_| PageTableError::OutOfMemory)?;
    }

    *KERNEL_ROOT.lock() = Some(root_frame);
    Ok(root_frame)
}

/// Activate virtual memory for riscv64 / aarch64.
///
/// # Safety
/// The root table MUST map the currently executing code and kernel stack.
#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
pub unsafe fn activate_paging(root_table_phys: PhysAddr) {
    // SAFETY: upheld by caller contract.
    let root_table = unsafe { &*(root_table_phys as *const PageTable) };
    // SAFETY: PageTable::activate writes SATP/TTBR0 using the table's physical address.
    unsafe { root_table.activate(); }
}

// ─── riscv64 + aarch64 + x86_64: map_page / unmap_page ──────────────────────
//
// On riscv64/aarch64 these delegate to PageTable::map / ::unmap.
// On x86_64 they use the HHDM-aware walk_create / walk_read walkers.

/// Map a virtual page to a physical frame in the kernel address space.
///
/// Returns `NotSupported` on bare-physical arches (riscv32, x86_32, arm32).
#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
pub fn map_page(
    allocator: &mut FrameAllocator,
    vaddr: VAddr,
    paddr: PhysAddr,
    flags: Flags,
) -> PagingResult<()> {
    let root_lock = KERNEL_ROOT.lock();
    if let Some(root_phys) = *root_lock {
        let root_table = unsafe { &mut *(root_phys as *mut PageTable) };
        let mut alloc_closure = || allocator.allocate_frame();
        root_table
            .map(vaddr, paddr, flags, &mut alloc_closure)
            .map_err(|_| PageTableError::OutOfMemory)?;
        Ok(())
    } else {
        Err(PageTableError::NotSupported)
    }
}

/// Unmap a virtual page from the kernel address space (clears the PTE).
/// Used to create guard pages. Returns Ok(()) if the page was not mapped.
///
/// Returns `NotSupported` on bare-physical arches.
#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
pub fn unmap_page(vaddr: VAddr) -> PagingResult<()> {
    let root_lock = KERNEL_ROOT.lock();
    if let Some(root_phys) = *root_lock {
        let root_table = unsafe { &mut *(root_phys as *mut PageTable) };
        match root_table.unmap(vaddr) {
            Ok(()) => Ok(()),
            Err(_) => Ok(()), // Not mapped — that is acceptable for guard-page creation
        }
    } else {
        Err(PageTableError::NotSupported)
    }
}

// ─── x86_64 paging ───────────────────────────────────────────────────────────
//
// Limine boot environment:
//   - Physical RAM is mapped at HHDM_BASE + phys (NOT identity-mapped).
//   - Kernel text/data live at 0xFFFFFFFF80000000 (higher half).
//   - MMIO (LAPIC, HPET, IOAPIC) is NOT in the HHDM; must be identity-mapped.
//
// Strategy for init_kernel_paging:
//   1. Allocate a new PML4 frame and zero it.
//   2. Copy PML4 entries [256..512] from Limine's CR3 — preserves kernel+HHDM.
//   3. Identity-map MMIO (VA = PA) so init_timers() can access 0xFED0_0000 etc.
//   4. Store root in KERNEL_ROOT and return phys address.
//
// The walk_create / walk_read helpers (in hal/arch/x86/src/x86_64/paging.rs)
// always use phys_to_virt_ptr() so intermediate tables are dereferenceable
// before and after the CR3 switch.

/// Build the kernel's own PML4 for x86_64.
///
/// Must be called after `set_phys_offset` (HHDM base) is stored in
/// `memory::frame` and after the HAL paging walker has the HHDM offset
/// set via `hal::paging::set_hhdm_offset`.
///
/// Preconditions:
/// - The frame allocator is initialised.
/// - `hal::paging::set_hhdm_offset(hhdm)` has been called.
///
/// # Panics
/// Panics on OOM (any frame allocation fails).
#[cfg(target_arch = "x86_64")]
pub fn init_kernel_paging_x86(
    allocator: &mut FrameAllocator,
) -> PagingResult<PhysAddr> {
    use hal::paging::{
        read_cr3, walk_create, walk_read,
        pte_flags_kernel_rw, pte_flags_mmio,
        PTE_PRESENT,
    };
    use crate::memory::frame::phys_to_virt;

    // 1. Allocate and zero a new PML4 frame.
    let pml4_phys = allocator.allocate_frame().ok_or(PageTableError::OutOfMemory)?;
    // SAFETY: pml4_phys is a freshly allocated 4KB frame; HHDM virt pointer is valid.
    unsafe { core::ptr::write_bytes(phys_to_virt(pml4_phys) as *mut u8, 0, PAGE_SIZE); }

    let pml4_virt = phys_to_virt(pml4_phys) as *mut u64;

    // 2. Copy higher-half PML4 entries [256..512] from Limine's active PML4.
    //    These entries cover 0xFFFF_8000_0000_0000..0xFFFF_FFFF_FFFF_FFFF and include:
    //      - the HHDM window (index 256..384 depending on Limine's choice)
    //      - the kernel text/data at 0xFFFFFFFF80000000 (index 511)
    //    Copying them wholesale is safe: they remain read-only during init.
    {
        // SAFETY: read_cr3() returns the physical address of the live Limine PML4.
        let limine_cr3 = unsafe { read_cr3() } as usize;
        let limine_pml4 = phys_to_virt(limine_cr3) as *const u64;
        // SAFETY: limine_pml4 points to a valid 512-entry PML4 (4096 bytes).
        // We only read entries [256..512] from it.
        unsafe {
            for i in 256usize..512 {
                let entry = core::ptr::read_volatile(limine_pml4.add(i));
                core::ptr::write_volatile(pml4_virt.add(i), entry);
            }
        }
    }

    // 3. Identity-map MMIO regions so the CPU can reach them at VA == PA after
    //    the CR3 switch.  These physical addresses are in the 0xFEC0_0000..0xFEE1_0000
    //    range which is below the 4GB boundary and NOT in the HHDM window.
    //
    //    IOAPIC:  0xFEC0_0000 (4 KB)
    //    HPET:    0xFED0_0000 (4 KB)   — init_timers() passes this as virt addr
    //    LAPIC:   0xFEE0_0000 (4 KB)
    let mmio_regions: &[(usize, usize)] = &[
        (0xFEC0_0000, 0xFEC0_0000 + PAGE_SIZE),
        (0xFED0_0000, 0xFED0_0000 + PAGE_SIZE),
        (0xFEE0_0000, 0xFEE0_0000 + PAGE_SIZE),
    ];
    for &(start, end) in mmio_regions {
        let mut va = start;
        while va < end {
            let mut alloc_fn = || allocator.allocate_frame();
            // SAFETY: pml4_virt points to our new PML4 (writable via HHDM).
            // walk_create allocates intermediate tables via alloc_fn.
            let pte_ptr = unsafe { walk_create(pml4_virt, va, &mut alloc_fn) }
                .ok_or(PageTableError::OutOfMemory)?;
            // SAFETY: pte_ptr is the leaf PTE slot returned by walk_create; safe to write.
            unsafe { core::ptr::write_volatile(pte_ptr, va as u64 | pte_flags_mmio()); }
            va += PAGE_SIZE;
        }
    }

    // Sanity: verify one entry was written correctly (first MMIO page).
    // SAFETY: pml4_virt is our valid new PML4.
    debug_assert!(
        unsafe { walk_read(pml4_virt as *const u64, 0xFEC0_0000) }
            .map(|e| e & PTE_PRESENT != 0)
            .unwrap_or(false),
        "IOAPIC identity-map sanity check failed"
    );

    log::info!("[kernel] x86_64 paging: kernel PML4 built");

    *KERNEL_ROOT.lock() = Some(pml4_phys);
    Ok(pml4_phys)
}

/// Activate paging on x86_64 by writing the PML4 physical address to CR3.
///
/// # Safety
/// The PML4 at `root_phys` must map the currently executing code, the kernel
/// stack, and any data the kernel needs immediately after this call.
/// An invalid mapping causes a triple-fault.
#[cfg(target_arch = "x86_64")]
pub unsafe fn activate_paging(root_phys: PhysAddr) {
    // SAFETY: caller guarantees root_phys is a valid, fully populated PML4 that
    // keeps the kernel alive after the CR3 switch.
    unsafe { hal::paging::write_cr3(root_phys as u64); }
    log::info!("[kernel] paging activated");
}

/// Map a 4KB page (x86_64).
///
/// `flags` must be raw x86_64 PTE flags (use `pte_flags_*` helpers from
/// `hal::paging`).  The function inserts the PTE into KERNEL_ROOT's PML4 and
/// issues `invlpg` to flush the TLB entry.
#[cfg(target_arch = "x86_64")]
pub fn map_page_x86(
    allocator: &mut FrameAllocator,
    vaddr: VAddr,
    paddr: PhysAddr,
    flags: u64,
) -> PagingResult<()> {
    use hal::paging::{walk_create, invlpg};
    use crate::memory::frame::phys_to_virt;

    let root_lock = KERNEL_ROOT.lock();
    let root_phys = (*root_lock).ok_or(PageTableError::NotSupported)?;
    let pml4_virt = phys_to_virt(root_phys) as *mut u64;

    let mut alloc_fn = || allocator.allocate_frame();
    // SAFETY: pml4_virt is the kernel's active PML4 (stored at boot).
    let pte_ptr = unsafe { walk_create(pml4_virt, vaddr, &mut alloc_fn) }
        .ok_or(PageTableError::OutOfMemory)?;
    // SAFETY: pte_ptr is the leaf PTE address; writing it installs the mapping.
    unsafe { core::ptr::write_volatile(pte_ptr, paddr as u64 | flags); }
    // SAFETY: invlpg flushes only the single TLB entry for vaddr.
    unsafe { invlpg(vaddr); }
    Ok(())
}

/// Unmap a 4KB page (x86_64).
///
/// Clears the leaf PTE and flushes the TLB entry. Returns Ok(()) even if the
/// page was already unmapped (idempotent — safe for guard-page creation).
#[cfg(target_arch = "x86_64")]
pub fn unmap_page_x86(vaddr: VAddr) -> PagingResult<()> {
    use hal::paging::{walk_read, walk_create, invlpg, PTE_PRESENT};
    use crate::memory::frame::phys_to_virt;

    let root_lock = KERNEL_ROOT.lock();
    let root_phys = match *root_lock {
        Some(p) => p,
        None => return Ok(()), // paging not yet active — no TLB entry to clear
    };
    let pml4_virt_const = phys_to_virt(root_phys) as *const u64;
    // SAFETY: pml4_virt_const is the kernel's active PML4.
    let existing = unsafe { walk_read(pml4_virt_const, vaddr) };
    if let Some(pte) = existing {
        if pte & PTE_PRESENT != 0 {
            // Walk again for a mutable pointer to the leaf PTE.
            let pml4_virt_mut = phys_to_virt(root_phys) as *mut u64;
            // SAFETY: walk_create on an existing path never allocates new frames;
            // alloc_fn is only called for absent intermediate tables (none here).
            let mut noop_alloc = || None::<usize>;
            if let Some(pte_ptr) = unsafe { walk_create(pml4_virt_mut, vaddr, &mut noop_alloc) } {
                // SAFETY: pte_ptr is the leaf PTE slot; clearing it unmaps the page.
                unsafe { core::ptr::write_volatile(pte_ptr, 0u64); }
                // SAFETY: invlpg flushes the TLB entry for this virtual address.
                unsafe { invlpg(vaddr); }
            }
        }
    }
    Ok(())
}

// ─── Generic map_page / unmap_page wrappers (called from kernel code) ────────
//
// These forward to the correct arch implementation. x86_64 versions take raw
// u64 PTE flags; callers that need the generic Flags type should use the
// riscv64/aarch64 versions above or convert flags themselves.
//
// For x86_64, map_page and unmap_page delegate to map_page_x86 /
// unmap_page_x86 using kernel-RW flags by default.  Callers needing
// user/exec/mmio flags must call map_page_x86 directly.

/// Map a page: generic entry point.
///
/// On riscv64/aarch64: uses the HAL-generic Flags parameter.
/// On x86_64: converts generic Flags to x86_64 PTE bits.
/// On bare-physical arches: returns NotSupported.
#[cfg(target_arch = "x86_64")]
pub fn map_page(
    allocator: &mut FrameAllocator,
    vaddr: VAddr,
    paddr: PhysAddr,
    flags: Flags,
) -> PagingResult<()> {
    use hal::paging::{PTE_WRITABLE, PTE_USER, PTE_NX, PTE_PRESENT};
    // Convert generic PageFlags to x86_64 PTE bits.
    // Note: cache-disable (PCD/MMIO) has no generic PageFlags equivalent;
    // callers needing MMIO mappings must use map_page_x86 with pte_flags_mmio().
    let bits = flags.bits();
    let mut pte_flags: u64 = PTE_PRESENT;
    if bits & hal::PageFlags::WRITE   != 0 { pte_flags |= PTE_WRITABLE; }
    if bits & hal::PageFlags::USER    != 0 { pte_flags |= PTE_USER; }
    if bits & hal::PageFlags::EXECUTE == 0 { pte_flags |= PTE_NX; }
    map_page_x86(allocator, vaddr, paddr, pte_flags)
}

/// Unmap a page: generic entry point.
///
/// On x86_64 delegates to unmap_page_x86 (idempotent).
/// On bare-physical arches: returns NotSupported.
#[cfg(target_arch = "x86_64")]
pub fn unmap_page(vaddr: VAddr) -> PagingResult<()> {
    unmap_page_x86(vaddr)
}

// ─── remap_range_user ────────────────────────────────────────────────────────

/// Remap a range of already-allocated pages with user (U/S=1) permissions.
///
/// Used for user stacks which sit in identity-mapped usable RAM.
/// On x86_64 we re-walk and rewrite the PTEs with USER|WRITABLE|NX.
#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
pub fn remap_range_user(start: PhysAddr, pages: usize) {
    let mut root_guard = KERNEL_ROOT.lock();
    if let Some(root_addr) = root_guard.as_mut() {
        let table = unsafe { &mut *(*root_addr as *mut hal::paging::PageTable) };
        let mut frame_guard = crate::memory::frame::FRAME_ALLOCATOR.lock();
        if let Some(allocator) = frame_guard.as_mut() {
            let mut alloc_closure = || allocator.allocate_frame();
            use hal::traits::PageTableTrait;
            let flags = PageFlags::from_bits(
                PageFlags::VALID | PageFlags::READ | PageFlags::WRITE
                    | PageFlags::EXECUTE | PageFlags::USER
                    | PageFlags::ACCESSED | PageFlags::DIRTY,
            );
            use hal::paging::PAGE_SIZE;
            for i in 0..pages {
                let addr = start + (i * PAGE_SIZE);
                let _ = table.map(addr, addr, flags, &mut alloc_closure)
                    .expect("Failed to map user stack page!");
            }
        }
    }
}

#[cfg(target_arch = "x86_64")]
pub fn remap_range_user(start: PhysAddr, pages: usize) {
    use hal::paging::pte_flags_user_rw;
    let mut frame_guard = crate::memory::frame::FRAME_ALLOCATOR.lock();
    if let Some(allocator) = frame_guard.as_mut() {
        for i in 0..pages {
            let addr = start + (i * PAGE_SIZE);
            // Map VA == PA (identity) with user-rw flags.
            let _ = map_page_x86(allocator, addr, addr, pte_flags_user_rw());
        }
    }
}

// ─── Bare-physical stubs ─────────────────────────────────────────────────────

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
pub fn remap_range_user(_start: PhysAddr, _pages: usize) {}

// ─── virt_to_phys ────────────────────────────────────────────────────────────

/// Translate a virtual address to its physical address by walking the kernel
/// page table.
///
/// Bare-physical arches (riscv32, x86_32, arm32): VA == PA.
/// Paged arches read the page-table root from the architecture register.
#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
pub fn virt_to_phys(vaddr: VAddr) -> Option<PhysAddr> {
    use hal::traits::PageTableTrait;

    #[cfg(target_arch = "riscv64")]
    {
        // SAFETY: `satp` CSR holds the page-table root PPN.
        let satp: usize;
        unsafe { core::arch::asm!("csrr {}, satp", out(reg) satp) };
        let root_ppn = satp & ((1 << 44) - 1);
        let root_phys = root_ppn << 12;
        if root_phys == 0 { return None; }
        // SAFETY: root_phys is the physical address of the active root PageTable.
        let root_table = unsafe { &*(root_phys as *const hal::PageTable) };
        return root_table.translate(vaddr);
    }
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: TTBR0_EL1 holds the root page table physical address.
        let ttbr0: usize;
        unsafe { core::arch::asm!("mrs {}, ttbr0_el1", out(reg) ttbr0, options(nomem, nostack)) };
        let root_phys = ttbr0 & !0xFFF;
        if root_phys == 0 { return None; }
        let root_table = unsafe { &*(root_phys as *const hal::PageTable) };
        return root_table.translate(vaddr);
    }
    #[allow(unreachable_code)]
    None
}

#[cfg(target_arch = "x86_64")]
pub fn virt_to_phys(vaddr: VAddr) -> Option<PhysAddr> {
    use hal::paging::{walk_read, PTE_PRESENT};
    use crate::memory::frame::phys_to_virt;
    let root_guard = KERNEL_ROOT.lock();
    let root_phys = (*root_guard)?;
    let pml4 = phys_to_virt(root_phys) as *const u64;
    // SAFETY: pml4 is the kernel's active PML4.
    let pte = unsafe { walk_read(pml4, vaddr) }?;
    if pte & PTE_PRESENT == 0 { return None; }
    Some(((pte & !0xFFF) as usize) | (vaddr & 0xFFF))
}

/// Bare-physical identity translation.
#[cfg(any(target_arch = "riscv32", target_arch = "x86", target_arch = "arm"))]
pub fn virt_to_phys(vaddr: VAddr) -> Option<PhysAddr> {
    Some(vaddr)
}

// ─── Page-fault handler (x86_64) ─────────────────────────────────────────────

/// HAL-visible entry point: called by the IDT error-code handler via the
/// `extern "Rust" fn vi_handle_page_fault` declaration in `hal/arch/x86/idt.rs`.
///
/// Decision tree:
///   - User-mode fault (error_code bit 2 = U/S set): look up the current task's
///     VMA list; if a region covers `va`, install the mapping and return so the
///     faulting instruction is retried.
///   - Otherwise (kernel fault or no VMA match): panic — this is a true
///     kernel bug or an unmapped user access.
///
/// # Safety
/// Must only be called from the #PF exception handler while the faulting
/// context is suspended on the interrupt stack. The caller must NOT hold any
/// spinlock that this function might try to acquire (KERNEL_ROOT, SCHEDULER,
/// FRAME_ALLOCATOR).
#[cfg(target_arch = "x86_64")]
#[no_mangle]
pub extern "Rust" fn vi_handle_page_fault(va: usize, error_code: u64) {
    use crate::task::SCHEDULER;

    // Bit 2 of the error code = U/S: fault originated from user mode.
    let user_fault = error_code & (1 << 2) != 0;

    if !user_fault {
        panic!(
            "[#PF kernel] va={:#x} error_code={:#x} — kernel-mode page fault",
            va, error_code
        );
    }

    // Try to find the faulting VA in the current task's VMA list.
    // We identify the current task via the scheduler's running-task pointer.
    let result: Option<(PhysAddr, u64)> = {
        let mut sched = SCHEDULER.lock();
        if let Some(s) = sched.as_mut() {
            // `current_task_mut()` returns the task currently marked Running
            // (the one whose exception context we are inside).
            s.current_task_mut().and_then(|task| {
                task.vma.find(va).map(|region| {
                    let offset = va - region.va_start;
                    let pa = region.pa_start + offset;
                    let flags = region.flags;
                    (pa, flags)
                })
            })
        } else {
            None
        }
    };

    match result {
        Some((paddr, pte_flags)) => {
            // Install the mapping on demand. For ELF-backed segments the ELF loader
            // (Phase 04) sets pa_start to the physical frame it copied the segment
            // into; we just wire it up here. For demand-allocated regions (Stack/Heap)
            // the ELF loader leaves pa_start = 0 and the fault handler allocates.
            //
            // Phase 01: VMA list is always empty, so this branch is unreachable
            // in practice — any user fault panics at the None branch above.
            let mut frame_guard = crate::memory::frame::FRAME_ALLOCATOR.lock();
            let alloc = frame_guard.as_mut()
                .unwrap_or_else(|| panic!("[#PF] frame allocator unavailable at va={:#x}", va));
            let effective_pa = if paddr == 0 {
                // Demand-allocate a new zeroed frame.
                alloc.allocate_frame()
                    .unwrap_or_else(|| panic!("[#PF] OOM allocating demand page at va={:#x}", va))
            } else {
                paddr
            };
            let _ = map_page_x86(alloc, va & !0xFFF, effective_pa & !0xFFF, pte_flags);
        }
        None => {
            panic!(
                "[#PF user] va={:#x} error_code={:#x} — no VMA covers this address",
                va, error_code
            );
        }
    }
}
