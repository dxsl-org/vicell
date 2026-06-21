//! Physical frame allocator for ViCell kernel.
//!
//! Manages physical memory frames (4KB pages) using a Bitmap Allocator.
//! This allows for O(1) allocation and deallocation (amortized) and frame reuse.

use crate::boot::MemoryMapEntry;
use crate::*;
use core::sync::atomic::{AtomicUsize, Ordering};

// Define PAGE_SIZE to avoid circular dependency with paging.rs
const PAGE_SIZE: usize = 4096;
const KERNEL_HEAP_PAGES: usize = 4096;

/// Physical-to-virtual address offset.
/// - RISC-V: 0 (identity-mapped before activate_paging, SATP disabled)
/// - x86_64: HHDM_BASE from Limine (RAM mapped at hhdm+phys, not identity)
static PHYS_OFFSET: AtomicUsize = AtomicUsize::new(0);

/// Set the physical-to-virtual offset. Must be called before `new_from_map`.
/// On x86_64, set to the Limine HHDM base address.
pub fn set_phys_offset(offset: usize) {
    PHYS_OFFSET.store(offset, Ordering::Relaxed);
}

/// Convert a physical address to the virtual address used to access it.
#[inline]
pub fn phys_to_virt(phys: usize) -> usize {
    phys + PHYS_OFFSET.load(Ordering::Relaxed)
}

/// Bitmap Frame Allocator
pub struct FrameAllocator {
    /// Start of usable memory managed by this allocator
    memory_start: PhysAddr,
    /// End of usable memory
    memory_end: PhysAddr,
    /// Total frames managed
    total_frames: usize,
    /// Bitmap storage (borrowed from reserved memory)
    bitmap: &'static mut [u64],
    /// Index of the last allocated frame (for next-fit search)
    last_alloc_index: usize,
}

impl FrameAllocator {
    /// Initialize allocator from memory map
    ///
    /// This function finds the largest usable memory region, reserves space for the bitmap
    /// at the beginning of that region, and initializes the allocator.
    pub fn new_from_map(entries: &[MemoryMapEntry]) -> Self {
        let mut best_start = 0;
        let mut best_end = 0;
        let mut max_len = 0;

        // 1. Find largest usable region
        for entry in entries {
            if entry.ty == crate::boot::MemoryType::Usable {
                if entry.length > max_len {
                    max_len = entry.length;
                    best_start = entry.base;
                    best_end = entry.base + entry.length;
                }
            }
        }

        // Align start to 4KB
        let aligned_start = (best_start + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let aligned_end = best_end & !(PAGE_SIZE - 1);
        let available_size = aligned_end - aligned_start;
        let total_frames = available_size / PAGE_SIZE;

        // 2. Calculate bitmap size
        // We need 1 bit per frame.
        // 1 u64 = 64 bits = 64 frames.
        // Bitmap size in u64s = (total_frames + 63) / 64
        let bitmap_u64_count = (total_frames + 63) / 64;
        let bitmap_size_bytes = bitmap_u64_count * 8;

        // 3. Place bitmap at the beginning of the region
        // We need to reserve enough *pages* for the bitmap
        let bitmap_pages = (bitmap_size_bytes + PAGE_SIZE - 1) / PAGE_SIZE;
        let bitmap_phys_addr = aligned_start;

        // 4. Create the bitmap slice.
        // On RISC-V, phys == virt (SATP disabled). On x86_64, Limine maps RAM
        // at HHDM_BASE+phys — physical addresses are NOT identity-mapped.
        // SAFETY: We own this memory region and we are single-threaded at init.
        let bitmap = unsafe {
            core::slice::from_raw_parts_mut(
                phys_to_virt(bitmap_phys_addr) as *mut u64,
                bitmap_u64_count,
            )
        };

        // 5. Initialize bitmap
        // Initially, we mark ALL frames as FREE (0).
        // Then we mark the frames used by the bitmap itself as USED (1).
        for i in 0..bitmap_u64_count {
            bitmap[i] = 0;
        }

        // 6. Adjust allocator start to after the bitmap
        // But wait, the bitmap index 0 corresponds to `aligned_start`.
        // So we just need to mark the first `bitmap_pages` frames as used.

        let mut allocator = Self {
            memory_start: aligned_start,
            memory_end: aligned_end,
            total_frames,
            bitmap,
            last_alloc_index: 0,
        };

        // Mark bitmap pages as used
        for i in 0..bitmap_pages {
            allocator.mark_used(i);
        }

        allocator
    }

    /// Allocate a physical frame
    pub fn allocate_frame(&mut self) -> Option<PhysAddr> {
        // Simple Next-Fit algorithm
        let start_index = self.last_alloc_index;

        // First pass: from last_alloc to end
        if let Some(idx) = self.find_free(start_index, self.total_frames) {
            self.mark_used(idx);
            self.last_alloc_index = idx + 1;
            return Some(self.frame_index_to_addr(idx));
        }

        // Second pass: from 0 to last_alloc
        if let Some(idx) = self.find_free(0, start_index) {
            self.mark_used(idx);
            self.last_alloc_index = idx + 1;
            return Some(self.frame_index_to_addr(idx));
        }

        None // OOM
    }

    /// Deallocate a physical frame
    pub fn deallocate_frame(&mut self, frame: PhysAddr) {
        if let Some(idx) = self.addr_to_frame_index(frame) {
            self.mark_free(idx);
            // Optimization: Reset last_alloc_index if we freed a lower index?
            // Maybe not needed for next-fit.
        } else {
            log::warn!("Attempted to free invalid frame: 0x{:X}", frame);
        }
    }

    // --- Helper bits ---

    fn find_free(&self, start_idx: usize, end_idx: usize) -> Option<usize> {
        let mut bit_idx = start_idx;
        while bit_idx < end_idx {
            let u64_idx = bit_idx / 64;
            let bit_offset = bit_idx % 64;

            let block = self.bitmap[u64_idx];

            // Optimization: Skip full blocks
            if block == !0 {
                // All 1s
                bit_idx = (u64_idx + 1) * 64;
                continue;
            }

            // Check if specific bit is 0
            if (block & (1u64 << bit_offset)) == 0 {
                return Some(bit_idx);
            }
            bit_idx += 1;
        }
        None
    }

    fn mark_used(&mut self, idx: usize) {
        let u64_idx = idx / 64;
        let bit_offset = idx % 64;
        self.bitmap[u64_idx] |= 1u64 << bit_offset;
    }

    fn mark_free(&mut self, idx: usize) {
        let u64_idx = idx / 64;
        let bit_offset = idx % 64;
        self.bitmap[u64_idx] &= !(1u64 << bit_offset);
    }

    fn frame_index_to_addr(&self, idx: usize) -> PhysAddr {
        self.memory_start + (idx * PAGE_SIZE)
    }

    fn addr_to_frame_index(&self, addr: PhysAddr) -> Option<usize> {
        if addr < self.memory_start || addr >= self.memory_end {
            return None;
        }
        Some((addr - self.memory_start) / PAGE_SIZE)
    }

    /// Get total available memory in bytes
    pub fn total_memory(&self) -> usize {
        self.total_frames * PAGE_SIZE
    }

    /// Get used memory in bytes (Approximate)
    pub fn used_memory(&self) -> usize {
        // Counting bits is expensive, just return total - free is better but we don't track free count.
        // For now, let's just return 0 or implement counting later.
        // This is mainly for stats.
        0
    }

    // ── Snapshot serialization accessors ──────────────────────────────────────

    /// Physical start address of the allocator's managed region.
    pub fn memory_start(&self) -> PhysAddr { self.memory_start }

    /// Physical end address (exclusive) of the allocator's managed region.
    pub fn memory_end(&self) -> PhysAddr { self.memory_end }

    /// Total number of 4096-byte frames managed by this allocator.
    pub fn total_frames(&self) -> usize { self.total_frames }

    /// Returns `true` if frame `idx` is currently allocated (in use).
    ///
    /// Used by the snapshot serializer to enumerate only the allocated frames,
    /// avoiding snapshotting free memory and reducing snapshot size.
    pub fn is_frame_allocated(&self, idx: usize) -> bool {
        if idx >= self.total_frames { return false; }
        let u64_idx = idx / 64;
        let bit_offset = idx % 64;
        (self.bitmap[u64_idx] >> bit_offset) & 1 != 0
    }

    /// Physical address of frame `idx`.
    pub fn frame_addr(&self, idx: usize) -> PhysAddr {
        self.frame_index_to_addr(idx)
    }

    /// Mark `n` consecutive frames starting at `start_idx` as allocated.
    ///
    /// Used by `allocate_guest_ram` after locating a free run so the final
    /// allocation is a single, atomic lock hold rather than n individual calls.
    pub fn mark_range_used(&mut self, start_idx: usize, n: usize) {
        debug_assert!(
            start_idx + n <= self.total_frames,
            "mark_range_used: frame range [{}, {}) out of bounds (total={})",
            start_idx, start_idx + n, self.total_frames,
        );
        for i in 0..n {
            self.mark_used(start_idx + i);
        }
    }

    /// Find `n` consecutive free frames and mark them all allocated.
    ///
    /// Returns the physical address of the first frame, or `None` when no
    /// contiguous run of `n` frames is available.  Linear O(frames × n) scan
    /// — acceptable for startup-time Grant allocations with n ≤ 16.
    pub fn allocate_contiguous(&mut self, n: usize) -> Option<PhysAddr> {
        if n == 1 {
            return self.allocate_frame();
        }
        let limit = self.total_frames.saturating_sub(n);
        'outer: for start in 0..=limit {
            for i in 0..n {
                if self.is_frame_allocated(start + i) {
                    continue 'outer;
                }
            }
            // Found n consecutive free frames.
            for i in 0..n {
                self.mark_used(start + i);
            }
            return Some(self.frame_index_to_addr(start));
        }
        None
    }
}

/// Global frame allocator
pub static FRAME_ALLOCATOR: crate::sync::Spinlock<Option<FrameAllocator>> =
    crate::sync::Spinlock::new(None);

/// Allocate N contiguous physical frames for guest VM RAM using a chunked scan.
///
/// Releases `FRAME_ALLOCATOR` every `PROBE_CHUNK` frames during the search phase
/// to keep lock-hold time bounded and prevent the RT watchdog from firing during
/// a 512 MiB (131 072-frame) contiguous search (Red-Team M2).
///
/// # TOCTOU
/// After locating a candidate run, the lock is re-acquired to re-verify and mark
/// all frames atomically.  Transparent on QEMU TCG (single CPU).  A production
/// SMP build would use a buddy allocator with a free-run index.
pub fn allocate_guest_ram(n_pages: usize) -> Option<PhysAddr> {
    const PROBE_CHUNK: usize = 256;

    let total = FRAME_ALLOCATOR.lock().as_ref()?.total_frames;
    let limit = total.saturating_sub(n_pages);
    let mut candidate = 0usize; // current candidate run start (frame index)
    let mut run_len = 0usize;   // confirmed free frames from candidate onward

    while candidate <= limit {
        let probe_from = candidate + run_len;
        if probe_from >= total {
            break;
        }

        // Probe up to PROBE_CHUNK frames under a bounded lock hold.
        let (chunk_free, first_used) = {
            let g = FRAME_ALLOCATOR.lock();
            let a = g.as_ref()?;
            let probe_end = (probe_from + PROBE_CHUNK).min(total);
            let mut free_cnt = 0usize;
            let mut used_at = None;
            for idx in probe_from..probe_end {
                if a.is_frame_allocated(idx) {
                    used_at = Some(idx);
                    break;
                }
                free_cnt += 1;
            }
            (free_cnt, used_at)
        }; // lock dropped — other kernel tasks may run

        run_len += chunk_free;

        if run_len >= n_pages {
            // Candidate run is long enough — re-verify and allocate under lock.
            let result = {
                let mut g = FRAME_ALLOCATOR.lock();
                let a = g.as_mut()?;
                let all_free = (0..n_pages).all(|i| !a.is_frame_allocated(candidate + i));
                if all_free {
                    a.mark_range_used(candidate, n_pages);
                    Some(a.frame_addr(candidate))
                } else {
                    None // race: another CPU grabbed a frame; restart
                }
            };
            if let Some(pa) = result {
                return Some(pa);
            }
            candidate += 1;
            run_len = 0;
            continue;
        }

        if let Some(used_idx) = first_used {
            candidate = used_idx + 1;
            run_len = 0;
        }
        // If chunk was all free but run_len < n_pages: loop to probe next chunk.
    }
    None
}
