# Phase 04 — TLSF Real-Time Heap

**Status**: 📋 PLANNED  
**Priority**: P1  
**Effort**: 2 days  
**Depends on**: Phase 02 (TaskPriority enum must exist)

---

## Context Links

- Global allocator: `kernel/src/memory/heap.rs` (linked_list_allocator)
- Kernel Cargo.toml: `kernel/Cargo.toml`
- Main init sequence: `kernel/src/main.rs` (heap init at ~line 153)
- TCB struct: `kernel/src/task/tcb.rs:115-155`
- Spawn from mem: `kernel/src/task.rs`
- TLSF spec: `docs/specs/02-memory.md:26-27`

---

## Overview

`linked_list_allocator` (the current `#[global_allocator]`) is O(n) worst-case for allocation — unacceptable for hard-RT tasks. The spec (`02-memory.md §2`) requires a dedicated TLSF (Two-Level Segregated Fit) pool for RealTime cells with O(1) guaranteed alloc/free.

**Approach:** Add `rlsf` as a second static allocator alongside the existing global heap. `#[global_allocator]` stays as `linked_list_allocator` (used by Normal/Background cells and kernel internals). RealTime cell stack frames are carved from the TLSF pool via explicit `rt_alloc()` / `rt_dealloc()` calls.

---

## Requirements

- `rlsf` is the only maintained no_std O(1)-guaranteed TLSF crate (verified via research)
- RT pool size: 256 KiB static backing store — sufficient for ~4 RT cells with 64 KiB stacks each
- `init_rt_heap()` called from `main.rs` after the main heap is initialized
- `rt_alloc(layout)` / `rt_dealloc(ptr, align)` are the only public API — no `#[global_allocator]` use
- RealTime cells have their `kernel_stack` and `user_stack` carved from the RT pool
- Normal/Background cells use the existing global heap (no change)
- RT pool OOM returns `Err(ViError::OutOfMemory)` at spawn time, not a panic

---

## Key Insights

### rlsf type parameters
`rlsf::Tlsf<'static, FLBitmapType, SLBitmapType, FLLEN, SLLEN>`:
- `FLLEN=20` → max block ≈ 1 MiB (log2(1MiB) = 20)
- `SLLEN=16` → 16 sublists per first-level class → worst-case fragmentation ≤ ~6%
- `FLBitmapType=u32, SLBitmapType=u16` → small bitmaps, no heap needed

```rust
type RtTlsf = rlsf::Tlsf<'static, u32, u16, 20, 16>;
```

### Thread safety
`rlsf::Tlsf` is not `Send` by design — callers must provide locking. ViCell wraps it in the existing `Spinlock<T>` which disables interrupts, providing safe exclusive access from both task and ISR context.

### Why not replace the global allocator
- Replacing would force ALL kernel allocations (BTreeMap, Vec, Box) through TLSF
- TLSF has higher per-allocation overhead than linked_list for tiny objects
- The two-tier design matches the spec exactly: RT pool for RT stacks, global heap for everything else

---

## Related Code Files

### Create
- `kernel/src/memory/rt_heap.rs` — TLSF pool implementation

### Modify
- `kernel/Cargo.toml` — add `rlsf = { version = "0.2", default-features = false }`
- `kernel/src/memory.rs` or `kernel/src/memory/mod.rs` — add `pub mod rt_heap`
- `kernel/src/main.rs` — call `memory::rt_heap::init()` after heap init
- `kernel/src/task.rs` — `spawn_from_mem()` for RT cells: use `rt_heap::alloc_stack()` instead of `Stack::new()`

---

## Implementation Steps

### Step 1 — Add rlsf dependency

```toml
# kernel/Cargo.toml
[dependencies]
rlsf = { version = "0.2", default-features = false }
```

### Step 2 — Create `kernel/src/memory/rt_heap.rs`

```rust
//! Real-time heap — a TLSF allocator for RealTime cell stacks.
//!
//! Provides O(1) worst-case alloc/dealloc, isolated from the normal
//! linked-list global heap.  Callers must use `rt_alloc`/`rt_dealloc`
//! directly; this pool is not the #[global_allocator].

use crate::sync::Spinlock;
use core::alloc::Layout;
use core::ptr::NonNull;
use rlsf::Tlsf;

/// Backing memory for the RT pool.
///
/// 256 KiB static store — enough for ~4 RealTime cells with 64 KiB stacks.
/// Increase if more concurrent RT cells are required.
static mut RT_POOL_MEM: [u8; 256 * 1024] = [0u8; 256 * 1024];

type RtTlsf = Tlsf<'static, u32, u16, 20, 16>;

static RT_HEAP: Spinlock<Option<RtTlsf>> = Spinlock::new(None);

/// Initialise the RT heap.  Must be called once after the main heap is ready.
/// Subsequent calls are no-ops.
pub fn init() {
    let mut guard = RT_HEAP.lock();
    if guard.is_some() {
        return;
    }

    let mut tlsf = RtTlsf::INIT;
    // SAFETY: RT_POOL_MEM is a static mut array; this function is called once
    // before any RT allocation.  The Tlsf borrows the slice for 'static.
    unsafe {
        tlsf.insert_free_block(&mut RT_POOL_MEM);
    }
    *guard = Some(tlsf);
    log::info!("[rt-heap] TLSF pool initialised ({} KiB)", RT_POOL_MEM.len() / 1024);
}

/// Allocate from the RT pool.
///
/// Returns `Err(())` on OOM — the caller converts this to `ViError::OutOfMemory`.
///
/// # Safety
/// The returned pointer is valid until `rt_dealloc` is called with the same
/// pointer and align.
pub unsafe fn rt_alloc(layout: Layout) -> Result<NonNull<u8>, ()> {
    RT_HEAP.lock()
        .as_mut()
        .expect("rt_heap not initialised — call memory::rt_heap::init() first")
        .allocate(layout)
        .ok_or(())
}

/// Deallocate memory previously returned by `rt_alloc`.
///
/// # Safety
/// `ptr` must have been returned by `rt_alloc` with the given `align`.
pub unsafe fn rt_dealloc(ptr: NonNull<u8>, align: usize) {
    if let Some(tlsf) = RT_HEAP.lock().as_mut() {
        tlsf.deallocate(ptr, align);
    }
}
```

### Step 3 — Initialise RT heap in main.rs

After the existing `memory::heap::init_heap(heap_start, heap_size)` call (around main.rs:154):
```rust
memory::rt_heap::init();
log::info!("RT heap initialized");
```

### Step 4 — Wire RT stacks for RealTime cells

In `spawn_from_mem()` (or the stack allocation helper it calls), check task priority and select allocator:

```rust
let kernel_stack = if priority >= api::TaskPriority::RealTime as u8 {
    // Allocate from TLSF RT pool for O(1) worst-case guarantee.
    Stack::new_from_rt_heap(KERNEL_STACK_SIZE)?
} else {
    Stack::new(KERNEL_STACK_SIZE)?  // global linked-list allocator
};
```

Add `Stack::new_from_rt_heap(size: usize) -> ViResult<Stack>` that calls `rt_heap::rt_alloc(Layout::from_size_align(size, 4096).unwrap())`.

The `Drop` impl for `Stack` must detect whether the memory came from the RT pool and call the matching dealloc. Tag with a bool field: `Stack { ptr, size, from_rt_heap: bool }`.

---

## Todo List

- [ ] Add `rlsf = { version = "0.2", default-features = false }` to `kernel/Cargo.toml`
- [ ] Create `kernel/src/memory/rt_heap.rs` (TLSF pool, `init()`, `rt_alloc()`, `rt_dealloc()`)
- [ ] Add `pub mod rt_heap` in `kernel/src/memory` module
- [ ] Call `memory::rt_heap::init()` in `kernel/src/main.rs`
- [ ] Add `Stack::new_from_rt_heap()` and `from_rt_heap: bool` tag to `Stack`
- [ ] Update `spawn_from_mem()` to use RT pool for RealTime cells
- [ ] Confirm `Stack::drop()` calls correct dealloc based on `from_rt_heap` tag
- [ ] `cargo check -p vicell-kernel` — no errors
- [ ] Integration test: RT cell spawn + exit; verify RT pool is not exhausted (no OOM)

---

## Success Criteria

- [ ] `rt_alloc(Layout::from_size_align(4096, 4096))` completes without unbounded scan
- [ ] `init_rt_heap()` log line appears in QEMU boot output
- [ ] RT cell stack allocated from RT pool; Normal cell stack from global heap (verified by log)
- [ ] RT pool correctly freed when RT cell exits (no leak on repeated spawn/exit cycles)
- [ ] All 65 integration tests pass

---

## Risk Assessment

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| `rlsf` MSRV mismatch | Low | Check against nightly-2026-05-01; rlsf 0.2 targets stable Rust |
| RT pool OOM on boot (too small) | Low | 256 KiB / 64 KiB per stack = 4 slots; only 1-2 RT cells expected |
| Stack double-free (wrong dealloc path) | Medium | `from_rt_heap` bool tag on Stack; `debug_assert` in Drop |
| `insert_free_block` called twice (init race) | Low | Spinlock + early-return `if guard.is_some()` prevents duplicate init |
