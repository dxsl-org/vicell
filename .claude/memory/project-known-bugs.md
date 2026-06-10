---
name: project-known-bugs
description: Pre-existing bugs in ViCell kernel that are tracked but not yet fixed
metadata:
  type: project
---

## Heap Size / Frame Count Mismatch (HIGH RISK)

**Location:** `kernel/src/main.rs` lines ~130-145

**Bug:** The kernel allocates 16MB of physical frames for the heap (4096 frames × 4KB) but then tells `init_heap` it has 64MB (`heap_size = 4096*4096*4`). The extra 48MB is a lie — the frame allocator may hand out frames inside that phantom arena, causing silent heap corruption.

Additionally, `allocate_frame()` uses next-fit and does NOT guarantee contiguous frames; `init_heap` assumes a contiguous arena starting at `heap_start`.

**Why discovered:** Code review during Phase 04/05 work (2026-05-29).

**Risk:** Silent memory corruption at runtime whenever heap reaches ~16MB or when the frame allocator issues a frame that collides with the heap arena.

**How to apply:** Before implementing any feature that relies on large heap allocations (Phase 13 VFS, Phase 15 NIC, Phase 17 Shell), fix the heap init to either (a) allocate the full 64MB of contiguous frames or (b) reduce `heap_size` to match allocated frames. Do not block existing phases on this, but track it.

## Double Stack Allocation in spawn_synthetic / user_hello (LOW IMPACT)

**Location:** `kernel/src/task/user_hello.rs::spawn()`, `kernel/src/task.rs::spawn_synthetic()`

**Bug:** Both functions call `super::spawn()` (which calls `Scheduler::spawn` → allocates a kernel + user stack), then immediately re-allocate new stacks and replace the originals. The first pair is wastefully allocated and immediately dropped.

**Why discovered:** Code review H1 during Phase 03 (2026-05-29).

**Risk:** Wastes 2 × STACK_PAGES × 2 frames (= 2 × 16 × 2 × 4KB = 256KB) on every synthetic spawn. Not a crash, not a correctness issue.

**How to apply:** Before implementing Phase 06 (External ELF loading), consider adding a lower-level `spawn_bare()` primitive to `Scheduler` that creates a task without allocating stacks, so the caller can supply its own. This avoids the wasteful double-allocation across all synthetic spawn paths.

## `a7 = 93` exit syscall (already fixed in this session's diff)

**Was:** `li a7, 93` in `libs/ostd/src/startup.rs` _start (Linux exit = 93)
**Fixed to:** `li a7, 60` which maps to `ViSyscall::Exit` in `libs/api/src/syscall.rs:16,55`
The previous value was silently rejected by the kernel dispatcher — any cell exit was a no-op.
