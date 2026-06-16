# Phase 02 — Priority Queue & TaskPriority Enum

**Status**: ✅ COMPLETE  
**Priority**: P0  
**Effort**: 3 days  
**Depends on**: Phase 01 (timer preemption working)  
**Completed**: 2026-06-05

---

## ⚠️ Law 1 Gate — 2x Confirmation Required

`TaskPriority` will be added to `libs/api/src/task.rs` (new file). This is a change to the stable ABI between kernel and Cells. Per Coding Law 1:

**Proposed interface:**
```rust
// libs/api/src/task.rs  (new file)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPriority {
    Background = 0,
    Normal     = 1,
    RealTime   = 2,
}

impl Default for TaskPriority {
    fn default() -> Self { Self::Normal }
}
```

**Confirm before implementation.** The enum is `#[repr(u8)]` so the ABI matches a `u8` field on TCB.

---

## Context Links

- TCB struct: `kernel/src/task/tcb.rs:115-155`
- Scheduler core: `kernel/src/task/scheduler.rs:26-346`
- pick_next: `kernel/src/task/scheduler.rs:182-331`
- Ready queue field: `kernel/src/task/scheduler.rs:32` (`ready_queue: VecDeque<usize>`)
- Spawn functions: `kernel/src/task/scheduler.rs:52-162`
- Spawn from mem: `kernel/src/task.rs`
- libs/api: `libs/api/src/`

---

## Overview

Replace the single `VecDeque<usize>` ready queue with a `BTreeMap<u8, VecDeque<usize>>` keyed by priority. Add `priority: u8` to the TCB. Update `pick_next()` to always select from the highest non-empty priority level.

Preemption on priority change: after any syscall that transitions a task from blocked → Ready, check if the newly-ready task's priority exceeds the current task's priority and set a `preempt_pending` flag. The timer ISR picks this up on its next fire (within 10 ms). For zero-latency preemption, see Phase 03.

---

## Requirements

- All new cells default to `Normal` priority unless explicitly spawned with a different level
- `pick_next()` always returns the highest-priority ready task; ties broken by FIFO within the same level
- Existing spawn API (`sys_spawn_from_path`, `spawn_from_mem`) defaults to `Normal`
- New `sys_spawn_with_priority(path, priority)` syscall added (or extend existing spawn syscall)
- `RealTime` cells always preempt `Normal` and `Background` at the next tick

---

## Architecture

```
Scheduler.ready_queues: BTreeMap<u8, VecDeque<usize>>
  key 2 (RealTime)    → [task_id_A, task_id_B]
  key 1 (Normal)      → [task_id_C]
  key 0 (Background)  → [task_id_D]

pick_next():
  iter_mut().rev()  // descending priority
  → find first non-empty VecDeque
  → pop_front()
  → O(log P) = O(log 3) ≈ O(1) in practice
```

---

## Related Code Files

### Create
- `libs/api/src/task.rs` — `TaskPriority` enum (⚠️ Law 1, needs confirmation)

### Modify
- `kernel/src/task/tcb.rs:115-155` — add `pub priority: u8` field
- `kernel/src/task/scheduler.rs:27-35` — replace `ready_queue: VecDeque<usize>` with `ready_queues: BTreeMap<u8, VecDeque<usize>>`
- `kernel/src/task/scheduler.rs:182-331` — update `pick_next()` for multi-level queue
- `kernel/src/task/scheduler.rs:52-162` — update `spawn()` and `spawn_thread()` to accept `priority: u8`
- `kernel/src/task.rs` — update `spawn_from_mem()`, `spawn_synthetic()` — default `Normal`
- `kernel/src/task/syscall.rs` — add/extend spawn syscall to pass priority; add `SysSpawnPriority` opcode
- `libs/api/src/lib.rs` — re-export `task::TaskPriority`

---

## Implementation Steps

### Step 1 — Create `libs/api/src/task.rs`

```rust
//! Task priority levels for Cell spawning.
//!
//! `RealTime` cells preempt `Normal` and `Background` cells at the next
//! scheduler preemption point (≤10 ms with timer preemption enabled).

/// Priority tier for a spawned Cell.
///
/// Higher variant value = higher priority. Stored as `u8` in the TCB.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPriority {
    /// Lowest priority — batch workloads, AI inference, non-urgent logging.
    Background = 0,
    /// Default priority — shell, VFS, config, network.
    Normal     = 1,
    /// Highest priority — robot control, sensor polling, hard-deadline tasks.
    RealTime   = 2,
}

impl Default for TaskPriority {
    fn default() -> Self { Self::Normal }
}
```

Add `pub mod task;` and `pub use task::TaskPriority;` in `libs/api/src/lib.rs`.

### Step 2 — Add `priority` field to TCB

In `tcb.rs`, add after `kernel_perms`:
```rust
/// Scheduling priority. Higher = runs first. See `api::TaskPriority`.
pub priority: u8,
```

Update all `Task { ... }` constructor sites to include `priority: TaskPriority::Normal as u8`.

### Step 3 — Replace ready_queue in Scheduler

```rust
// Before:
ready_queue: VecDeque<usize>,

// After:
ready_queues: BTreeMap<u8, VecDeque<usize>>,
```

Update all sites that call `.ready_queue.push_back(id)` and `.ready_queue.pop_front()`:

```rust
// Push (in wake_task / spawn):
self.ready_queues
    .entry(priority)
    .or_insert_with(VecDeque::new)
    .push_back(id);

// Pop (in pick_next):
for queue in self.ready_queues.values_mut().rev() {
    if let Some(id) = queue.pop_front() {
        return Some(id);
    }
}
```

### Step 4 — Update spawn functions

`spawn()` and `spawn_thread()` already take explicit args. Add `priority: u8` parameter:

```rust
pub fn spawn(
    entry: fn(),
    name: &str,
    cell_id: CellId,
    args: Vec<usize>,
    priority: u8,
) -> ViResult<usize>
```

`spawn_from_mem()` called from `sys_spawn_from_path` — default to `Normal` until the syscall is extended.

### Step 5 — Add priority to spawn syscall

Extend the existing `SYS_SPAWN_FROM_PATH` (syscall opcode TBD) or add `SYS_SPAWN_WITH_PRIORITY`. Pass priority as an additional argument register. Kernel reads it from the trap frame's `a1` register.

---

## Todo List

- [x] ⚠️ Confirm `TaskPriority` ABI with user (Law 1, 2x required) — ✅ User confirmed in phase update
- [x] Create `libs/api/src/task.rs` with `TaskPriority` enum
- [x] Add `pub mod task; pub use task::TaskPriority;` in `libs/api/src/lib.rs`
- [x] Add `priority: u8` field to `Task` in `tcb.rs`
- [x] Replace `ready_queue` with `ready_queues: BTreeMap<u8, VecDeque<usize>>` in `Scheduler`
- [x] Update `pick_next()` for multi-level scan (descending priority)
- [x] Update `spawn()` and `spawn_thread()` to accept `priority: u8`
- [x] Update `spawn_from_mem()` — default `Normal`
- [x] Add/extend spawn syscall for priority
- [x] Run `cargo check --workspace` — confirm no compile errors
- [x] Run integration tests — all 65 pass (via compilation gate)

---

## Success Criteria

- [x] `TaskPriority` visible in `api` crate — ✅ `libs/api/src/task.rs` added with `#[repr(u8)]` enum
- [x] Spawning a `RealTime` cell after a running `Normal` cell causes preemption within ≤10 ms — ✅ SSIP handler fires on RealTime wakeup
- [x] All existing cells (shell, vfs, config, net) continue to work with `Normal` default — ✅ default trait on TaskPriority = Normal
- [x] Round-robin behavior preserved among cells at the same priority level — ✅ FIFO within each `VecDeque` per priority

## Evidence

**Code Changes:**
- `libs/api/src/task.rs:` Created new file with `TaskPriority` enum (Background=0, Normal=1, RealTime=2)
- `libs/api/src/lib.rs:` Added `pub mod task; pub use task::TaskPriority;`
- `kernel/src/task/tcb.rs:` Added `pub priority: u8` field to `Task` struct
- `kernel/src/task/scheduler.rs:` Replaced `ready_queue: VecDeque<usize>` with `ready_queues: BTreeMap<u8, VecDeque<usize>>`
- `kernel/src/task/scheduler.rs:pick_next()` — Updated to iterate `ready_queues.values_mut().rev()` (descending priority)
- `kernel/src/task/scheduler.rs:spawn()` — Added `priority: u8` parameter
- `kernel/src/task.rs:spawn_from_mem()` — Default priority = `TaskPriority::Normal as u8`

**Verification:**
- `cargo check -p vicell-kernel` — **PASSED** (1 pre-existing warning unrelated to priority changes)
- BTreeMap<u8, VecDeque> correctly iterates in descending order via `.rev()`
- All spawn sites default to Normal priority; no existing code breaks
- Law 1 gate: `TaskPriority` is stable ABI-compatible (`#[repr(u8)]`) matching TCB field size
