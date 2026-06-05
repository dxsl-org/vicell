//! Real-time heap — an O(1) TLSF allocator for RealTime cell stacks.
//!
//! Isolated from the global `linked_list_allocator` heap so that RealTime cells
//! always receive bounded-latency allocation, even when the Normal heap is under
//! pressure from AI workloads.
//!
//! Call `init()` once after the main heap is ready.  Then use `alloc` / `dealloc`
//! directly; this pool is NOT the `#[global_allocator]`.

use crate::sync::Spinlock;
use core::alloc::Layout;
use core::ptr::NonNull;

/// TLSF first-level length: 20 → max block ≈ 1 MiB.
const FLLEN: usize = 20;
/// TLSF second-level length: 16 → worst-case internal fragmentation ≈ 6%.
const SLLEN: usize = 16;

type RtTlsf = rlsf::Tlsf<'static, u32, u16, FLLEN, SLLEN>;

/// Backing memory for the RT pool.
///
/// 256 KiB static store — enough for ~4 RealTime cells with 64 KiB stacks each.
/// `MaybeUninit<u8>` matches `insert_free_block`'s expected slice element type.
/// `align(8)` satisfies TLSF block-header alignment requirements.
#[repr(C, align(8))]
struct RtPoolMem([core::mem::MaybeUninit<u8>; 256 * 1024]);

static mut RT_POOL_MEM: RtPoolMem =
    RtPoolMem([core::mem::MaybeUninit::uninit(); 256 * 1024]);

static RT_HEAP: Spinlock<Option<RtTlsf>> = Spinlock::new(None);

/// Initialise the RT heap.
///
/// Must be called once after `memory::heap::init_heap()`.  Subsequent calls
/// are no-ops (idempotent).
pub fn init() {
    let mut guard = RT_HEAP.lock();
    if guard.is_some() {
        return;
    }

    // SAFETY: RtTlsf (rlsf::Tlsf) is valid when zero-initialised — all-zero
    // bitmaps represent an empty pool with no free blocks.
    let mut tlsf: RtTlsf = unsafe { core::mem::zeroed() };
    // SAFETY: RT_POOL_MEM is a static array initialised to zero.  This function
    // is called once (guarded by the Spinlock check above) before any RT cell
    // allocation, so no aliased mutable reference exists.
    unsafe {
        tlsf.insert_free_block(&mut RT_POOL_MEM.0[..]);
    }
    *guard = Some(tlsf);
    log::info!("[rt-heap] TLSF pool ready — 256 KiB, O(1) alloc/dealloc");
}

/// Allocate `layout` bytes from the RT pool.
///
/// Returns `None` on OOM; callers should convert this to `ViError::OutOfMemory`.
///
/// # Safety
/// The returned pointer is valid and correctly aligned until `dealloc` is called
/// with the same pointer and `layout.align()`.
pub unsafe fn alloc(layout: Layout) -> Option<NonNull<u8>> {
    RT_HEAP
        .lock()
        .as_mut()
        .expect("rt_heap::alloc called before rt_heap::init")
        .allocate(layout)
}

/// Deallocate memory previously returned by `rt_heap::alloc`.
///
/// # Safety
/// `ptr` must have been returned by `rt_heap::alloc` with the given `align`.
/// Calling this with a pointer from the global heap is undefined behaviour.
pub unsafe fn dealloc(ptr: NonNull<u8>, align: usize) {
    if let Some(tlsf) = RT_HEAP.lock().as_mut() {
        tlsf.deallocate(ptr, align);
    }
}
