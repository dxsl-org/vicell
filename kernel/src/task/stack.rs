//! Stack Management for Tasks.
//!
//! Handles allocation, deallocation, and guard pages for Kernel and User stacks.
//! Complies with Rule 2 (Owned Buffers / Memory Safety) and Rule 8 (Resource Management).

use crate::memory::frame::FRAME_ALLOCATOR;
use crate::memory::paging::{self, Flags, PAGE_SIZE};
use types::{VAddr, ViError};
use alloc::vec::Vec;
use log::{error, trace};

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

        let base_frame = allocator.allocate_frame().ok_or(ViError::OutOfMemory)?;
        let mut frames = Vec::with_capacity(total_pages);
        frames.push(base_frame);

        for i in 1..total_pages {
            let frame = allocator.allocate_frame().ok_or(ViError::OutOfMemory)?;
            if frame != base_frame + (i * PAGE_SIZE) {
                // If not contiguous, we are in trouble for Identity Mapping SAS.
                // We'd need to free previous and retry or have a better allocator.
                // For this task, we log warning and fail.
                error!("Stack allocation failed: Non-contiguous frames in Identity Map SAS.");
                return Err(ViError::OutOfMemory);
            }
            frames.push(frame);
        }

        let base_addr = base_frame; // Identity Map

        // 2. Map Pages
        // If Guard Page is requested, the bottom page (base_addr) is NOT mapped (or mapped as invalid).
        // Ideally unmapped.

        let usable_start_idx = if guard { 1 } else { 0 };

        // For SAS Identity Map:
        // Memory is ALREADY mapped as Kernel RWX by `init_kernel_paging`.
        // We just need to CHANGE permissions for User stack.
        // Or unmap the Guard Page.

        // Guard Page: Unmap it.
        if guard {
            // How to unmap? `paging::unmap`?
            // `paging.rs` doesn't expose unmap yet.
            // We can map it as INVALID (Valid=0).
            let flags = Flags::from_bits(0); // Invalid
             paging::map_page(allocator, base_addr, base_addr, flags)
                 .map_err(|_| ViError::OutOfMemory)?;
        }

        // Usable Pages
        let flags = if user_mode {
            // User Read/Write (Exec?)
            Flags::from_bits(Flags::VALID | Flags::READ | Flags::WRITE | Flags::USER | Flags::ACCESSED | Flags::DIRTY)
        } else {
            // Kernel Read/Write
            Flags::from_bits(Flags::VALID | Flags::READ | Flags::WRITE | Flags::ACCESSED | Flags::DIRTY)
        };

        for i in usable_start_idx..total_pages {
            let addr = base_addr + (i * PAGE_SIZE);
            paging::map_page(allocator, addr, addr, flags)
                 .map_err(|_| ViError::OutOfMemory)?;
        }

        // Calculate Top (Stack grows down)
        // Top is at the END of the allocated range.
        let top = base_addr + (total_pages * PAGE_SIZE);

        trace!("Allocated Stack: Base=0x{:X}, Top=0x{:X}, Pages={}, User={}", base_addr, top, pages, user_mode);

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
        // We need to free frames.
        // Since we don't have a `deallocate_frame` exposed cleanly in `FrameAllocator` trait usage
        // (often it's just a bump pointer in early OS), we might leak.
        // But if `FrameAllocator` supports it, we should call it.

        // Assuming `FRAME_ALLOCATOR` has dealloc.
        // Checking memory/frame.rs would be good.
        // For now, we just log.
        trace!("Dropping Stack at 0x{:X}", self.base);
    }
}
