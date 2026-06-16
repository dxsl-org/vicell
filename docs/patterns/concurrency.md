# Concurrency Patterns
> Part of [ViCell Patterns](../patterns.md)

## Spinlock with Interrupt Safety

**Intent**: Protect shared data with automatic interrupt state management.

```rust
pub struct Spinlock<T> {
    lock: AtomicBool,
    data: UnsafeCell<T>,
    saved_interrupt_state: AtomicBool,
}

impl<T> Spinlock<T> {
    pub fn lock(&self) -> SpinlockGuard<T> {
        let was_enabled = hal::interrupts_enabled();
        hal::disable_interrupts();                          // save + disable
        while self.lock.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }
        SpinlockGuard { lock: self, saved_state: was_enabled }
    }
}

impl<T> Drop for SpinlockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.lock.store(false, Ordering::Release);
        if self.saved_state { hal::enable_interrupts(); }  // restore
    }
}
```

**Usage**:
```rust
static SCHEDULER: Spinlock<Option<Scheduler>> = Spinlock::new(None);

pub fn schedule() {
    let mut sched = SCHEDULER.lock();  // interrupts disabled, restored on drop
    sched.as_mut().unwrap().run();
}
```

**Why**: Naive spinlock deadlocks if interrupt handler tries to acquire the same lock.
Interrupt-safe spinlock prevents this via RAII state save/restore.

## Global Singleton with Late Initialization

**Intent**: Global state initialized after boot, not at compile time.

```rust
static INSTANCE: Spinlock<Option<MySubsystem>> = Spinlock::new(None);

pub fn init() {
    *INSTANCE.lock() = Some(MySubsystem::new());
}

// For shared (non-Copy) types:
static INSTANCE: Spinlock<Option<Arc<MySubsystem>>> = Spinlock::new(None);

pub fn get_instance() -> Option<Arc<MySubsystem>> {
    INSTANCE.lock().as_ref().map(Arc::clone)
}
```

**Why `Option`**: Kernel boots before subsystems initialize; `None` is valid initial state.
Cannot use `const fn` for complex initialization.

## Priority Scheduler (Phase 25 — Planned)
> Learn from: [RTIC v2](https://github.com/rtic-rs/rtic) `rtic-sw-pass/src/`

```rust
pub enum TaskPriority {
    RealTime   = 0,   // robot control, interrupt handlers — never preempted
    Normal     = 1,   // shell, apps, network
    Background = 2,   // bench, LLM inference, batch
}
```

- Higher-priority tasks preempt via RISC-V software interrupt (SWI), not timer tick
- TLSF allocator reserved for `RealTime` tasks (O(1) guaranteed)
- RT cells pinned to core 0 via `spawn_pinned(0)` — immune to work stealing
