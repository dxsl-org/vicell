//! Heap allocator for ViCell kernel.
//!
//! Wraps `linked_list_allocator::LockedHeap` in a `QuotaAlloc` that charges
//! every allocation to the currently-executing Cell's quota.  The kernel
//! itself (CellId = 0) is unlimited.  A Cell that exceeds its quota receives
//! a null pointer from `alloc()` — no panic, no system halt.

use core::alloc::{GlobalAlloc, Layout};
use linked_list_allocator::LockedHeap;

struct QuotaAlloc {
    inner: LockedHeap,
}

// SAFETY: QuotaAlloc delegates to LockedHeap which handles its own
// interior mutability safely.  cell_quota operations are also thread-safe.
unsafe impl GlobalAlloc for QuotaAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let cell = crate::task::scheduler::current_cell_id();
        if !crate::memory::cell_quota::charge(cell, layout.size()) {
            // Cell quota exceeded — return null, no panic.
            return core::ptr::null_mut();
        }
        let ptr = self.inner.alloc(layout);
        if ptr.is_null() {
            // Inner heap OOM — refund the charge we already applied.
            crate::memory::cell_quota::refund(cell, layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        crate::memory::cell_quota::refund(
            crate::task::scheduler::current_cell_id(),
            layout.size(),
        );
        self.inner.dealloc(ptr, layout);
    }
}

#[global_allocator]
static ALLOCATOR: QuotaAlloc = QuotaAlloc { inner: LockedHeap::empty() };

/// Initialise the kernel heap.
///
/// # Safety
/// Must be called exactly once after physical memory is mapped.
pub unsafe fn init_heap(heap_start: usize, heap_size: usize) {
    ALLOCATOR.inner.lock().init(heap_start as *mut u8, heap_size);
}

/// Allocator error handler
#[alloc_error_handler]
fn alloc_error_handler(layout: core::alloc::Layout) -> ! {
    log::error!("allocation error: {:?}", layout);
    // Panic recovery is not possible for OOM, but we loop to avoid double-panics
    // if the panic handler tries to allocate.
    loop {
        #[cfg(target_arch = "x86_64")]
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)) };
        #[cfg(not(target_arch = "x86_64"))]
        unsafe { core::arch::asm!("wfi") };
    }
}
