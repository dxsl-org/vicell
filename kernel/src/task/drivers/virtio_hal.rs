use core::alloc::Layout;
use core::ptr::NonNull;
use virtio_drivers::{BufferDirection, Hal, PhysAddr};

/// VirtIO HAL adapter for ViOS.
///
/// Assumes identity mapping (VAddr == PAddr) throughout, which holds for the
/// kernel's current single-address-space model.  Revisit if an HHDM or IOMMU
/// is introduced: every `paddr = ptr as usize` line must become
/// `paddr = ptr as usize - HHDM_OFFSET`.
pub struct VirtioHal;

// SAFETY: VirtioHal holds no state — all methods are stateless function calls.
unsafe impl Hal for VirtioHal {
    fn dma_alloc(pages: usize, _direction: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        let layout = Layout::from_size_align(pages * 4096, 4096)
            .expect("VirtIO DMA layout must be valid — pages > 0 and align is power-of-two");

        // SAFETY: layout is non-zero size and properly aligned (4096-byte alignment
        // satisfies VirtIO virtqueue alignment requirement of ≥16 bytes).
        let ptr = unsafe { alloc::alloc::alloc(layout) };

        if ptr.is_null() {
            // The Hal trait cannot signal failure (it must return an address),
            // so we cannot propagate OOM to the caller. Panic rather than spin
            // silently forever: a kernel panic prints a diagnosable message and
            // halts, whereas an infinite spin looks like an unexplained hang
            // (this was exactly the symptom when the GPU framebuffer alloc
            // outgrew the heap).
            panic!("[virtio] DMA alloc OOM: {} pages ({} KB) requested", pages, pages * 4);
        }

        // SAFETY: ptr is non-null and points to `pages * 4096` bytes — safe to zero.
        unsafe { core::ptr::write_bytes(ptr, 0, layout.size()) };

        // Identity mapping: physical address == virtual address.
        let paddr = ptr as usize;
        log::trace!("[virtio] DMA alloc {} pages at V:{:p} P:0x{:X}", pages, ptr, paddr);

        // SAFETY: ptr is non-null (checked above).
        (paddr, unsafe { NonNull::new_unchecked(ptr) })
    }

    unsafe fn dma_dealloc(_paddr: PhysAddr, vaddr: NonNull<u8>, pages: usize) -> i32 {
        let layout = Layout::from_size_align(pages * 4096, 4096)
            .expect("dma_dealloc layout must match dma_alloc — invariant upheld by virtio-drivers");
        // SAFETY: vaddr was returned by dma_alloc for the same layout; virtio-drivers
        // guarantees dma_dealloc is called at most once per dma_alloc.  Using vaddr
        // (not paddr) keeps correctness when HHDM is introduced (paddr != vaddr).
        unsafe { alloc::alloc::dealloc(vaddr.as_ptr(), layout) };
        0
    }

    unsafe fn mmio_phys_to_virt(paddr: PhysAddr, _size: usize) -> NonNull<u8> {
        // SAFETY: identity mapping — paddr is a valid MMIO address mapped by
        // init_kernel_paging (CLINT/PLIC/UART/VirtIO explicit block).
        NonNull::new(paddr as *mut u8).expect("VirtIO MMIO address must be non-zero")
    }

    unsafe fn share(buffer: NonNull<[u8]>, _direction: BufferDirection) -> PhysAddr {
        let vaddr = buffer.as_ptr() as *mut u8 as usize;
        // Translate virtual → physical using the kernel page table.
        // ViOS SAS does not guarantee VA==PA for all mappings (ELF segments are
        // mapped at their load VA but allocated to arbitrary physical frames).
        // Heap-allocated buffers and kernel-stack frames happen to be identity-
        // mapped, but VFS BSS / static sector buffers are NOT — DMA must use the
        // real physical address to avoid writing to the wrong physical location.
        crate::memory::paging::virt_to_phys(vaddr).unwrap_or(vaddr)
    }

    unsafe fn unshare(_paddr: PhysAddr, _buffer: NonNull<[u8]>, _direction: BufferDirection) {
        // Identity mapping: no IOMMU to flush, no cache coherency action needed.
    }
}
