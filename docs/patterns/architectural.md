# Architectural Patterns
> Part of [Cellos Patterns](../patterns.md)

## Layered Architecture

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
- No skipping layers (Cells cannot directly access HAL)
- Each layer has a well-defined interface

**Example** (File I/O):
```rust
ostd::fs::open("/file.txt", OpenMode::Read)?;  // Cell calls ostd wrapper
  ↓
syscall::syscall3(ViSyscall::Open, ...);        // ostd makes syscall
  ↓
kernel::syscall::handle_syscall(ViSyscall::Open, args);  // Kernel handles
  ↓
fs::VIFS1.open(path, mode);                    // Kernel uses FS impl
```

## Trait-Based Abstraction

**Intent**: Define interfaces as traits, implementations as structs.

**When to Use**: Any functionality that might have multiple implementations.

```rust
// libs/api/src/fs.rs
pub trait ViFileSystem: Send + Sync {
    fn open(&self, path: &str, mode: OpenMode) -> ViResult<Box<dyn ViFile>>;
    fn mkdir(&self, path: &str) -> ViResult<()>;
}

// kernel/src/fs/fat.rs
impl ViFileSystem for FatFs {
    fn open(&self, path: &str, mode: OpenMode) -> ViResult<Box<dyn ViFile>> { ... }
}

// Usage — trait object, swap implementations freely
let fs: Arc<dyn ViFileSystem + Send + Sync> = Arc::new(FatFs::new());
```

**Benefits**: multiple implementations, dynamic Cell loading, testable with mocks, clear API contracts.

## Dependency Injection

**Intent**: Pass dependencies explicitly rather than hard-coding them.

```rust
// Bad: tight coupling
pub fn spawn_task() {
    let scheduler = GLOBAL_SCHEDULER.lock();
    scheduler.add_task(...);
}

// Good: trait-based injection — can swap implementations
pub fn spawn_task<S: TaskScheduler>(scheduler: &mut S) -> ViResult<usize> {
    scheduler.add_task(...)
}
```

**Benefits**: easier to test (inject mocks), flexible, clearer dependency graph.
