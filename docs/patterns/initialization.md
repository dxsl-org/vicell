# Initialization Patterns
> Part of [ViCell Patterns](../patterns.md)

## Two-Phase Initialization

**Intent**: Separate construction (compile time) from initialization (runtime after dependencies ready).

**Phase 1 — Static construction**:
```rust
static SUBSYSTEM: Spinlock<Option<Subsystem>> = Spinlock::new(None);
```

**Phase 2 — Runtime initialization** (after dependencies available):
```rust
pub fn init() {
    let subsystem = Subsystem::new().expect("Failed to create subsystem");
    *SUBSYSTEM.lock() = Some(subsystem);
}
```

**Boot sequence example**:
```rust
fn kmain() {
    uart::init();       // no dependencies — first
    memory::init();     // depends on boot info
    fs::init();         // depends on memory
    task::init();       // depends on memory
    scheduler::init();  // depends on task
    spawn_init_cell();  // depends on all above
}
```

## Init Function per Module

**Convention**: Every module exports a top-level `init()`. Kernel calls them in dependency order.

```rust
// kernel/src/memory/memory.rs
pub fn init(boot_info: &dyn BootInfo) {
    frame::init(boot_info.memory_map());
    paging::init();
    heap::init();
}

// kernel/src/task/task.rs
pub fn init() {
    scheduler::init();
}
```

## Panic Recovery per Cell (Phase 26 — Planned)
> Learn from: [Tock OS process_standard.rs](https://github.com/tock/tock)

```rust
// Kernel wraps every Cell dispatch with catch_unwind
// Panic in Cell → kill that Cell only, kernel + others continue
let result = catch_unwind(|| cell.dispatch(msg));
if result.is_err() {
    cell_registry.mark_poisoned(cell_id);
    cell_registry.reload_from_disk(cell_id);  // hot-reload
}
```

Currently wired only at kernel panic boundary — needs extension to every Cell dispatch path.
