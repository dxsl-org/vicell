//! Stack Management for Tasks.
//!
//! Handles allocation, deallocation, and guard pages for Kernel and User stacks.
//! Complies with Rule 2 (Owned Buffers / Memory Safety) and Rule 8 (Resource Management).

use crate::memory::frame::FRAME_ALLOCATOR;
use crate::memory::paging::{self, Flags, PAGE_SIZE};
use alloc::vec::Vec;
use log::{error, trace};
use types::{VAddr, ViError};

/// Represents an allocated Stack.
/// Implements Drop to automatically free pages.
#[derive(Debug)]
pub struct Stack {
    /// Base address (lowest address) of the allocated range.
    /// This includes the guard page at the bottom if present.
    pub base: VAddr,
    /// Number of usable pages (excluding guard page).
    pub pages: usize,
    /// Whether this stack has a guard page.
    pub has_guard: bool,
    /// Top of the stack (initial SP).
    pub top: VAddr,
}

impl Stack {
    /// Allocate a new Kernel Stack.
    /// - `pages`: Number of usable pages.
    /// - Uses `FRAME_ALLOCATOR` to get contiguous physical frames.
    /// - Maps them as RWX (Kernel).
    /// - Adds a Guard Page at the bottom (Unmapped).
    pub fn new_kernel(pages: usize) -> Result<Self, ViError> {
        Self::allocate(pages, true, false)
    }

    /// Allocate a new User Stack.
    /// - `pages`: Number of usable pages.
    /// - Maps them as USER RWX.
    /// - Adds a Guard Page at the bottom (Unmapped).
    pub fn new_user(pages: usize) -> Result<Self, ViError> {
        Self::allocate(pages, true, true)
    }

    /// Internal allocation logic.
    fn allocate(pages: usize, guard: bool, user_mode: bool) -> Result<Self, ViError> {
        let total_pages = if guard { pages + 1 } else { pages };

        let mut frame_guard = FRAME_ALLOCATOR.lock();
        let allocator = frame_guard.as_mut().ok_or(ViError::OutOfMemory)?;

        // 1. Allocate contiguous frames
        // Note: Our simple allocator returns single frames.
        // We need contiguous VIRTUAL memory.
        // In Identity Mapping (SAS), Physical Contiguity = Virtual Contiguity.
        // So we need contiguous physical frames if we rely on simple pointer arithmetic.
        // However, `paging::map_page` maps arbitrary PAddr to VAddr.
        // BUT, our current `frame::FrameAllocator` (bitmap) might not guarantee contiguous frames.
        // And we don't have a virtual memory allocator (VMA) yet.
        //
        // TEMPORARY SOLUTION:
        // We assume we can get contiguous frames OR we are mapping to Identity.
        // For now, we try to allocate one by one.
        // Wait, if we use Identity Mapping for Kernel, we MUST have contiguous physical frames.
        //
        // If we are mapping User Stack, we can map arbitrary frames to contiguous Virtual Addresses?
        // No, we don't have a Virtual Address Allocator yet.
        // We are using Identity Mapping for everything currently (SAS).
        // So we MUST find a contiguous run of physical pages.
        //
        // Our `FrameAllocator` (likely a simple bump or bitmap) needs to support range allocation.
        // Looking at `kernel/src/memory/frame.rs` (not read yet, but assuming simple).
        //
        // Let's assume we can just call `allocate_frame` N times and check contiguity?
        // No, that's brittle.
        //
        // Let's rely on the fact that currently `allocate_frame` is likely linear.
        //
        // Refactoring: We will allocate the FIRST frame as base.
        // Then we hope subsequent calls are contiguous. If not, we panic/fail for now
        // (until VMA is implemented).

        let base_frame = allocator.allocate_contiguous(total_pages).ok_or(ViError::OutOfMemory)?;
        
        let base_addr = base_frame; // Identity Map

        // 2. Map Pages
        // If Guard Page is requested, the bottom page (base_addr) is NOT mapped (or mapped as invalid).
        // Ideally unmapped.

        let usable_start_idx = if guard { 1 } else { 0 };

        // SAS identity map: all RAM is already identity-mapped RWX by
        // init_kernel_paging. The usable pages are re-mapped below, then the guard
        // frame (base_addr) is unmapped after the loop so an overflow traps.

        // Usable Pages
        let flags = if user_mode {
            // User Read/Write (Exec?)
            Flags::from_bits(
                Flags::VALID
                    | Flags::READ
                    | Flags::WRITE
                    | Flags::USER
                    | Flags::ACCESSED
                    | Flags::DIRTY,
            )
        } else {
            // Kernel Read/Write
            Flags::from_bits(
                Flags::VALID | Flags::READ | Flags::WRITE | Flags::ACCESSED | Flags::DIRTY,
            )
        };

        for i in usable_start_idx..total_pages {
            let addr = base_addr + (i * PAGE_SIZE);
            paging::map_page(allocator, addr, addr, flags).map_err(|_| ViError::OutOfMemory)?;
        }

        // Guard page: drop the bottom frame's pre-existing identity mapping so a
        // stack overflow (a write below base_addr+PAGE_SIZE) faults instead of
        // silently corrupting the neighbouring frame. The spawn paths zero only
        // the usable pages (skipping base_addr), so nothing legitimately writes to
        // the guard frame. The frame stays owned by this Stack (freed in Drop);
        // only its PTE is cleared. unmap_page locks KERNEL_ROOT (not FRAME_ALLOCATOR,
        // which we still hold) — no deadlock.
        if guard {
            if paging::unmap_page(base_addr).is_err() {
                // Non-fatal: stack is still usable, just unguarded. Loud so a
                // silently-unprotected stack is never mistaken for a guarded one.
                error!("Stack guard NOT active: unmap of guard frame 0x{:X} failed", base_addr);
            } else {
                paging::tlb_flush_all();
            }
        }

        // Calculate Top (Stack grows down)
        // Top is at the END of the allocated range.
        let top = base_addr + (total_pages * PAGE_SIZE);

        trace!(
            "Allocated Stack: Base=0x{:X}, Top=0x{:X}, Pages={}, User={}",
            base_addr,
            top,
            pages,
            user_mode
        );

        Ok(Stack {
            base: base_addr,
            pages,
            has_guard: guard,
            top,
        })
    }
}

impl Drop for Stack {
    fn drop(&mut self) {
        trace!("Dropping Stack at 0x{:X}", self.base);

        let total_pages = if self.has_guard {
            self.pages + 1
        } else {
            self.pages
        };

        // Restore each frame to the BOOT identity mapping (kernel RWX) before
        // returning it to the allocator. This is load-bearing in the SAS model:
        // the cell loader zeroes a freshly-allocated frame through its identity
        // address (`phys_to_virt(frame)` == frame on RISC-V, elf.rs), so EVERY
        // free frame must be identity-mapped. Stack::new unmaps the guard frame
        // (overflow protection) and maps usable frames with USER flags; without
        // this restore, a freed guard frame stays unmapped → the next owner's
        // BSS memset store-faults, and a freed USER frame carries stale perms →
        // wrong-page reads (garbage WAD). Unmap-then-map lands every frame in a
        // clean, uniform kernel-RWX PTE regardless of its prior state.
        let kernel_rwx = paging::Flags::from_bits(
            paging::Flags::VALID
                | paging::Flags::READ
                | paging::Flags::WRITE
                | paging::Flags::EXECUTE
                | paging::Flags::ACCESSED
                | paging::Flags::DIRTY,
        );
        let mut frame_guard = FRAME_ALLOCATOR.lock();
        if let Some(allocator) = frame_guard.as_mut() {
            for i in 0..total_pages {
                let frame = self.base + (i * PAGE_SIZE);
                let _ = paging::unmap_page(frame);
                let _ = paging::map_page(allocator, frame, frame, kernel_rwx);
                allocator.deallocate_frame(frame);
            }

            paging::tlb_flush_all();
        }
    }
}

/// Physical frames mapped for a cell's ELF segments, recorded as `(vaddr, frame)`
/// at load time so they can be reclaimed when the cell dies.
///
/// `Stack::drop` only frees stacks; without this a cell's code/data frames leak
/// on every death (a supervised service restarted repeatedly would grow to OOM).
/// Segment frames are allocated exclusively for the cell by `load_segments`
/// (IPC/shared buffers use separate frames), so freeing them on death is safe.
#[derive(Debug)]
pub struct CellSegments {
    pages: alloc::vec::Vec<(types::VAddr, types::PhysAddr)>,
    /// VA base allocated by `va_alloc::alloc_cell_va` for PIE cells; `0` for
    /// fixed-VA cells.  Returned to the allocator's free list on drop.
    pie_va_base: usize,
}

impl CellSegments {
    pub fn new(pages: alloc::vec::Vec<(types::VAddr, types::PhysAddr)>, pie_va_base: usize) -> Self {
        Self { pages, pie_va_base }
    }

    /// Unmap this cell's segment VAs immediately at death — WITHOUT freeing the
    /// frames (those are freed lazily when the zombie is reaped, in `drop`).
    ///
    /// Frees the address space right away so (a) a respawn can reuse the fixed VA
    /// and (b) the load-time overwrite guard (`load_segments`) only ever observes
    /// LIVE cells' (and kernel MMIO) mappings, never a dead-but-unreaped cell's.
    /// Locks only `KERNEL_ROOT` (a leaf), so it is safe under the SCHEDULER lock.
    pub fn eager_unmap(&self) {
        let mut unmapped_any = false;
        for &(vaddr, frame) in &self.pages {
            // Only unmap a VA that still resolves to OUR frame (it won't if a
            // respawn already re-pointed it — leave the new mapping intact).
            if paging::virt_to_phys(vaddr) == Some(frame) {
                let _ = paging::unmap_page(vaddr);
                unmapped_any = true;
            }
        }
        if unmapped_any {
            paging::tlb_flush_all();
        }
    }
}

impl Drop for CellSegments {
    fn drop(&mut self) {
        let mut frame_guard = FRAME_ALLOCATOR.lock();
        if let Some(allocator) = frame_guard.as_mut() {
            for &(vaddr, frame) in &self.pages {
                // Only unmap if this VA still resolves to OUR frame. Cells load at
                // fixed VAs, so a supervised cell respawned at the same VA before we
                // are reaped will have re-pointed this VA at the NEW instance's frame
                // — unmapping it then would crash the new cell. Skip the unmap in
                // that case; the old frame is still ours to free either way.
                if paging::virt_to_phys(vaddr) == Some(frame) {
                    let _ = paging::unmap_page(vaddr);
                }
                allocator.deallocate_frame(frame);
            }
        }
        // Return the PIE VA slot to the allocator so it can be reused.
        if self.pie_va_base != 0 {
            crate::loader::va_alloc::free_cell_va(self.pie_va_base);
        }
    }
}
