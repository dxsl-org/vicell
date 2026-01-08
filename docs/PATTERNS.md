# ViOS Design Patterns

> **Common patterns, idioms, and architectural decisions in the ViOS codebase**

## Table of Contents

1. [Overview](#overview)
2. [Architectural Patterns](#architectural-patterns)
3. [Concurrency Patterns](#concurrency-patterns)
4. [Memory Management Patterns](#memory-management-patterns)
5. [Error Handling Patterns](#error-handling-patterns)
6. [API Design Patterns](#api-design-patterns)
7. [Initialization Patterns](#initialization-patterns)
8. [Testing Patterns](#testing-patterns)
9. [Anti-Patterns to Avoid](#anti-patterns-to-avoid)

---

## Overview

ViOS employs specific design patterns that align with its Cellular SAS architecture and safety goals. Understanding these patterns is crucial for writing idiomatic ViOS code.

### Pattern Philosophy

- **Safety First**: Leverage Rust's type system for correctness
- **Zero-Copy**: Minimize data movement via ownership transfer
- **Explicit over Implicit**: Make behavior clear and predictable
- **Fail Fast**: Detect errors early, handle gracefully
- **Portable**: Work across 32-bit and 64-bit architectures

---

## Architectural Patterns

### Pattern: Layered Architecture

**Intent**: Separate concerns into distinct layers with clear boundaries.

**Structure**:
```
┌──────────────────────────────────────┐
│         Cells (User Space)           │  ← Applications, Services, Drivers
├──────────────────────────────────────┤
│         Kernel (Nano Kernel)         │  ← Core OS functionality
├──────────────────────────────────────┤
│    HAL (Hardware Abstraction)        │  ← Architecture-specific code
├──────────────────────────────────────┤
│           Hardware                   │  ← Physical devices
└──────────────────────────────────────┘
```

**Rules**:
- Upper layers depend on lower layers only
- No skipping layers (e.g., Cells cannot directly access HAL)
- Each layer has a well-defined interface

**Example** (File I/O):
```rust
// Cell calls ostd wrapper
ostd::fs::open("/file.txt", OpenMode::Read)?;
  ↓
// ostd makes syscall
syscall::syscall3(ViSyscall::Open, ...);
  ↓
// Kernel handles syscall
kernel::syscall::handle_syscall(ViSyscall::Open, args);
  ↓
// Kernel uses filesystem implementation
fs::VIFS1.open(path, mode);
```

### Pattern: Trait-Based Abstraction

**Intent**: Define interfaces as traits, implementations as structs.

**When to Use**: For any functionality that might have multiple implementations.

**Example**:
```rust
// libs/api/src/fs.rs - Define interface
pub trait ViFileSystem: Send + Sync {
    fn open(&self, path: &str, mode: OpenMode) -> ViResult<Box<dyn ViFile>>;
    fn mkdir(&self, path: &str) -> ViResult<()>;
}

// kernel/src/fs/fat.rs - Implement for FAT32
pub struct FatFs { /* ... */ }

impl ViFileSystem for FatFs {
    fn open(&self, path: &str, mode: OpenMode) -> ViResult<Box<dyn ViFile>> {
        // FAT32-specific implementation
    }
}

// Usage with trait object
let fs: Arc<dyn ViFileSystem + Send + Sync> = Arc::new(FatFs::new());
```

**Benefits**:
- ✅ Multiple implementations without code changes
- ✅ Cells can load/unload dynamically
- ✅ Testing with mock implementations
- ✅ Clear API contracts

### Pattern: Dependency Injection

**Intent**: Pass dependencies explicitly rather than hard-coding them.

**Example** (Task creation):
```rust
// Bad: Hard-coded dependency
pub fn spawn_task() {
    let scheduler = GLOBAL_SCHEDULER.lock();  // Tight coupling
    scheduler.add_task(...);
}

// Good: Dependency injection
pub fn spawn_task(scheduler: &mut Scheduler) -> ViResult<usize> {
    scheduler.add_task(...)  // Loose coupling
}

// Better: Trait-based injection
pub fn spawn_task<S: TaskScheduler>(scheduler: &mut S) -> ViResult<usize> {
    scheduler.add_task(...)  // Can swap implementations
}
```

**Benefits**:
- ✅ Easier to test (inject mocks)
- ✅ More flexible (swap implementations)
- ✅ Clearer dependencies

---

## Concurrency Patterns

### Pattern: Spinlock with Interrupt Safety

**Intent**: Protect shared data with automatic interrupt state management.

**Implementation**:
```rust
// kernel/src/sync.rs
pub struct Spinlock<T> {
    lock: AtomicBool,
    data: UnsafeCell<T>,
    saved_interrupt_state: AtomicBool,
}

impl<T> Spinlock<T> {
    pub fn lock(&self) -> SpinlockGuard<T> {
        // 1. Save current interrupt state
        let was_enabled = hal::interrupts_enabled();

        // 2. Disable interrupts
        hal::disable_interrupts();

        // 3. Acquire lock (spin if needed)
        while self.lock.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }

        // 4. Return guard
        SpinlockGuard {
            lock: self,
            saved_state: was_enabled,
        }
    }
}

impl<T> Drop for SpinlockGuard<'_, T> {
    fn drop(&mut self) {
        // Release lock
        self.lock.lock.store(false, Ordering::Release);

        // Restore interrupt state
        if self.saved_state {
            hal::enable_interrupts();
        }
    }
}
```

**Usage**:
```rust
static SCHEDULER: Spinlock<Option<Scheduler>> = Spinlock::new(None);

pub fn schedule() {
    let mut sched = SCHEDULER.lock();  // Interrupts disabled automatically
    sched.as_mut().unwrap().run();
    // Lock released here, interrupts restored
}
```

**Why This Pattern**:
- ❌ Naive spinlock: Can deadlock if interrupt handler tries same lock
- ✅ Interrupt-safe spinlock: Prevents deadlock by disabling interrupts
- ✅ RAII: Automatic cleanup via Drop

### Pattern: Global Singleton with Late Initialization

**Intent**: Global state that's initialized after boot.

**Implementation**:
```rust
static INSTANCE: Spinlock<Option<MySubsystem>> = Spinlock::new(None);

pub fn init() {
    let mut inst = INSTANCE.lock();
    *inst = Some(MySubsystem::new());
}

pub fn get_instance() -> Option<MySubsystem> {
    INSTANCE.lock().clone()
}
```

**Why `Option`**:
- Kernel boots before subsystems initialize
- Cannot use `const fn` for complex initialization
- `Option::None` is a valid initial state

**Alternative** (For non-Copy types):
```rust
static INSTANCE: Spinlock<Option<Arc<MySubsystem>>> = Spinlock::new(None);

pub fn get_instance() -> Option<Arc<MySubsystem>> {
    INSTANCE.lock().as_ref().map(Arc::clone)
}
```

---

## Memory Management Patterns

### Pattern: Owned Buffers for Async

**Intent**: Avoid lifetime issues in async code by transferring ownership.

**Rule**: Never pass `&mut [u8]` across `await` points in SAS.

**Bad**:
```rust
async fn process(data: &mut [u8]) -> ViResult<()> {
    some_async_operation().await;  // ❌ data reference invalid here!
    data[0] = 42;  // Potential use-after-free
    Ok(())
}
```

**Good**:
```rust
async fn process(mut data: Box<[u8]>) -> ViResult<Box<[u8]>> {
    some_async_operation().await;  // ✅ data ownership preserved
    data[0] = 42;  // Safe!
    Ok(data)
}
```

**Why**:
- In SAS, pointers remain valid, but borrow checker cannot track across await
- Ownership transfer makes it explicit and safe

### Pattern: Arc for Shared Resources

**Intent**: Share immutable data across tasks without copying.

**Example**:
```rust
// Filesystem shared across all tasks
static FILESYSTEM: Spinlock<Option<Arc<dyn ViFileSystem + Send + Sync>>>
    = Spinlock::new(None);

pub fn register_filesystem(fs: impl ViFileSystem + 'static) {
    let fs_arc = Arc::new(fs);
    *FILESYSTEM.lock() = Some(fs_arc);
}

pub fn get_filesystem() -> Option<Arc<dyn ViFileSystem + Send + Sync>> {
    FILESYSTEM.lock().as_ref().map(Arc::clone)
}

// Usage in multiple tasks
let fs = get_filesystem().unwrap();
fs.open("/file.txt", OpenMode::Read)?;  // Shared, no copying
```

**When to Use**:
- Data accessed by multiple tasks
- Data is expensive to copy
- Immutable or interior mutability (Mutex/RwLock)

### Pattern: RAII for Resource Cleanup

**Intent**: Automatically release resources when they go out of scope.

**Example** (File handles):
```rust
pub struct FileHandle {
    file: Box<dyn ViFile + Send + Sync>,
}

impl Drop for FileHandle {
    fn drop(&mut self) {
        log::trace!("Closing file handle");
        // Cleanup happens automatically
    }
}

// Usage
{
    let file = FileHandle::new(fs.open("/file.txt")?);
    file.read(...)?;
}  // File automatically closed here
```

**Example** (Lease):
```rust
pub struct Lease {
    id: usize,
}

impl Drop for Lease {
    fn drop(&mut self) {
        // Revoke lease when dropped
        kernel::ipc::revoke_lease(self.id);
    }
}
```

**Benefits**:
- ✅ No resource leaks
- ✅ Exception-safe (panic-safe)
- ✅ Clear ownership semantics

---

## Error Handling Patterns

### Pattern: Result-Based Error Handling

**Intent**: Use `Result<T, E>` for recoverable errors.

**Type Alias**:
```rust
// libs/types/src/lib.rs
pub type ViResult<T> = Result<T, ViError>;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViError {
    NotFound,
    OutOfMemory,
    InvalidArgument,
    // ...
}
```

**Usage**:
```rust
pub fn allocate_frame() -> ViResult<PAddr> {
    let mut allocator = FRAME_ALLOCATOR.lock();
    allocator.as_mut()
        .ok_or(ViError::NotInitialized)?
        .allocate()
        .ok_or(ViError::OutOfMemory)
}
```

**Pattern: Early Return with `?`**:
```rust
fn complex_operation() -> ViResult<()> {
    let resource1 = acquire_resource1()?;  // Early return on error
    let resource2 = acquire_resource2()?;
    process(resource1, resource2)?;
    Ok(())
}
```

### Pattern: Error Context

**Intent**: Provide context when propagating errors.

**Example**:
```rust
fn load_config() -> ViResult<Config> {
    let data = fs::read_file("/etc/config.toml")
        .map_err(|e| {
            log::error!("Failed to read config file: {:?}", e);
            ViError::ConfigLoadFailed
        })?;

    parse_toml(&data)
        .map_err(|e| {
            log::error!("Failed to parse config: {:?}", e);
            ViError::ConfigParseFailed
        })
}
```

### Pattern: Panic for Unrecoverable Errors

**Intent**: Use `panic!` for programming errors and invariant violations.

**When to Panic**:
- Kernel internal invariants violated
- Hardware error that cannot be recovered
- "Should never happen" scenarios

**Example**:
```rust
pub fn schedule_next() {
    let mut sched = SCHEDULER.lock();
    let scheduler = sched.as_mut()
        .expect("Scheduler not initialized");  // Should never happen

    scheduler.pick_next_task()
        .expect("No runnable tasks");  // Should always have idle task
}
```

**Note**: Cells should avoid `panic!` when possible, use `Result` instead.

---

## API Design Patterns

### Pattern: Builder Pattern

**Intent**: Construct complex objects step-by-step.

**Example** (Task builder):
```rust
pub struct TaskBuilder {
    name: Option<String>,
    cell_id: CellId,
    entry_point: VAddr,
    stack_size: usize,
}

impl TaskBuilder {
    pub fn new(cell_id: CellId, entry_point: VAddr) -> Self {
        Self {
            name: None,
            cell_id,
            entry_point,
            stack_size: DEFAULT_STACK_SIZE,
        }
    }

    pub fn name(mut self, name: String) -> Self {
        self.name = Some(name);
        self
    }

    pub fn stack_size(mut self, size: usize) -> Self {
        self.stack_size = size;
        self
    }

    pub fn build(self) -> ViResult<Task> {
        Task::new(
            self.cell_id,
            self.name.unwrap_or_default(),
            self.entry_point,
            self.stack_size
        )
    }
}

// Usage
let task = TaskBuilder::new(cell_id, entry_point)
    .name("my_task".to_string())
    .stack_size(128 * 1024)
    .build()?;
```

### Pattern: Newtype Pattern

**Intent**: Type-safe wrappers around primitive types.

**Example**:
```rust
// libs/types/src/lib.rs
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VAddr(pub usize);

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PAddr(pub usize);

// Cannot accidentally mix virtual and physical addresses
fn map_page(vaddr: VAddr, paddr: PAddr) {  // Type-safe!
    // vaddr and paddr are distinct types
}

// Compiler prevents:
map_page(PAddr(0x8000_0000), VAddr(0x1000));  // ❌ Wrong order
```

### Pattern: Facade Pattern

**Intent**: Simplify complex subsystem with unified interface.

**Example** (HAL Core):
```rust
// hal/core/src/lib.rs
#[cfg(feature = "riscv64")]
pub use hal_riscv::rv64::*;

#[cfg(feature = "aarch64")]
pub use hal_arm::aarch64::*;

#[cfg(feature = "x86_64")]
pub use hal_x86::x86_64::*;

// Kernel just uses hal::
use hal::Arch;  // Automatically correct for target architecture
```

---

## Initialization Patterns

### Pattern: Two-Phase Initialization

**Intent**: Separate construction from initialization for complex subsystems.

**Phase 1: Construction** (at compile time or early boot):
```rust
static SUBSYSTEM: Spinlock<Option<Subsystem>> = Spinlock::new(None);
```

**Phase 2: Initialization** (after dependencies ready):
```rust
pub fn init() {
    log::info!("Initializing subsystem");

    let subsystem = Subsystem::new().expect("Failed to create subsystem");
    *SUBSYSTEM.lock() = Some(subsystem);

    log::info!("Subsystem initialized");
}
```

**Usage**:
```rust
// In kernel boot sequence
fn kmain() {
    // Phase 1: Early initialization
    uart::init();          // No dependencies
    memory::init();        // Depends on boot info

    // Phase 2: Late initialization
    fs::init();            // Depends on memory
    task::init();          // Depends on memory
    scheduler::init();     // Depends on task
}
```

### Pattern: Init Function per Module

**Convention**: Every module exports an `init()` function.

```rust
// kernel/src/memory/mod.rs
pub fn init(boot_info: &dyn BootInfo) {
    log::info!("Initializing memory subsystem");
    frame::init(boot_info.memory_map());
    paging::init();
    heap::init();
}

// kernel/src/task/mod.rs
pub fn init() {
    log::info!("Initializing task subsystem");
    scheduler::init();
}
```

---

## Testing Patterns

### Pattern: Unit Tests with Mocks

**Intent**: Test kernel code in isolation.

**Example**:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Mock allocator for testing
    struct MockAllocator {
        next_addr: PAddr,
    }

    impl MockAllocator {
        fn allocate(&mut self) -> Option<PAddr> {
            let addr = self.next_addr;
            self.next_addr = PAddr(self.next_addr.0 + PAGE_SIZE);
            Some(addr)
        }
    }

    #[test]
    fn test_stack_allocation() {
        let mut allocator = MockAllocator {
            next_addr: PAddr(0x8000_0000),
        };

        let stack = allocate_stack(&mut allocator).unwrap();
        assert!(stack.0 >= 0x8000_0000);
    }
}
```

### Pattern: Architecture Validation Tests

**Intent**: Verify API contracts and invariants.

**Example**:
```rust
// tests/architecture-validation/step1_contract_testing.rs

#[test]
fn test_filesystem_contract() {
    // Test that all ViFileSystem implementations satisfy contract
    let fs = FatFs::new();

    // Contract: open() should return error for nonexistent file
    assert!(fs.open("/nonexistent.txt", OpenMode::Read).is_err());

    // Contract: mkdir() should create directory
    fs.mkdir("/test_dir").unwrap();
    assert!(fs.open("/test_dir", OpenMode::Read).unwrap().is_dir());
}
```

---

## Anti-Patterns to Avoid

### Anti-Pattern: Unsafe Cells

**Problem**: Using `unsafe` in Cell code.

❌ **Don't**:
```rust
// cells/apps/myapp/src/main.rs
#![no_std]
#![no_main]
// Missing: #![forbid(unsafe_code)]

fn main() {
    unsafe {
        let ptr = 0x8000_0000 as *mut u8;
        *ptr = 0;  // Violates safety guarantees
    }
}
```

✅ **Do**:
```rust
// cells/apps/myapp/src/main.rs
#![no_std]
#![no_main]
#![forbid(unsafe_code)]  // Enforced at compile time

fn main() {
    // Use safe APIs only
    ostd::println!("Hello");
}
```

### Anti-Pattern: mod.rs Files

**Problem**: Using outdated Rust module style.

❌ **Don't**:
```
memory/
└── mod.rs         # Old style
```

✅ **Do**:
```
memory.rs          # Main module
memory/            # Submodules
├── frame.rs
└── paging.rs
```

### Anti-Pattern: Hardcoded Sizes

**Problem**: Assuming pointer size.

❌ **Don't**:
```rust
const KERNEL_BASE: u64 = 0xFFFF_FFFF_8000_0000;  // Breaks on 32-bit
```

✅ **Do**:
```rust
const KERNEL_BASE: VAddr = VAddr(0x8000_0000);  // Portable
```

### Anti-Pattern: Global Mutable State without Synchronization

**Problem**: Race conditions.

❌ **Don't**:
```rust
static mut COUNTER: usize = 0;  // Unsafe, races possible

pub fn increment() {
    unsafe {
        COUNTER += 1;  // Race condition
    }
}
```

✅ **Do**:
```rust
static COUNTER: Spinlock<usize> = Spinlock::new(0);

pub fn increment() {
    let mut count = COUNTER.lock();
    *count += 1;
}
```

### Anti-Pattern: Borrowed Buffers for Async

**Problem**: Lifetime issues in SAS.

❌ **Don't**:
```rust
async fn send_data(data: &[u8]) -> ViResult<()> {
    async_send(data).await  // Lifetime violation
}
```

✅ **Do**:
```rust
async fn send_data(data: Box<[u8]>) -> ViResult<()> {
    async_send(data).await  // Ownership transferred, safe
}
```

---

## Pattern Decision Matrix

**When to use which pattern:**

| Scenario | Pattern | Why |
|----------|---------|-----|
| Global state | Spinlock + Option | Thread-safe, late init |
| Shared resource | Arc + dyn Trait | Multiple owners |
| Resource cleanup | RAII + Drop | Automatic, exception-safe |
| API abstraction | Trait | Multiple implementations |
| Async operations | Owned buffers (Box) | Avoid lifetime issues |
| Error handling | Result | Recoverable errors |
| Invariant violation | panic! | Unrecoverable errors |
| Complex construction | Builder | Many optional parameters |
| Type safety | Newtype | Prevent misuse |

---

## Summary

ViOS patterns are designed for:
- **Safety**: Leverage Rust's type system
- **Performance**: Zero-copy, minimal overhead
- **Portability**: Work across architectures
- **Maintainability**: Clear, idiomatic code

**Key Takeaways**:
1. Use `Spinlock` for all global state
2. Transfer ownership (Box/Arc) not borrows (&)
3. Implement `Drop` for resources
4. `Result` for errors, `panic!` for bugs
5. Traits for abstractions, newtypes for safety

Follow these patterns and your code will fit naturally into the ViOS ecosystem.

---

**Last Updated**: 2026-01-07
**Version**: 0.2.0
**Maintainer**: ViOS Team
