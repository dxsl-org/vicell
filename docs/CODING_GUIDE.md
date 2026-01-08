# ViOS Coding Guide

> **How to write code that fits ViOS conventions and architectural principles**

## Table of Contents

1. [Overview](#overview)
2. [The Golden Rules](#the-golden-rules)
3. [Project Structure](#project-structure)
4. [Naming Conventions](#naming-conventions)
5. [Code Organization](#code-organization)
6. [Writing Kernel Code](#writing-kernel-code)
7. [Writing Cell Code](#writing-cell-code)
8. [Memory Management Patterns](#memory-management-patterns)
9. [Error Handling](#error-handling)
10. [Async Programming](#async-programming)
11. [Testing](#testing)
12. [Code Review Checklist](#code-review-checklist)

---

## Overview

ViOS is **not a traditional Unix/Linux OS**. It uses a **Cellular Single Address Space (SAS)** architecture with **Language-Based Isolation (LBI)**. Writing code for ViOS requires understanding these fundamental differences.

### Key Principles

1. **Cellular, Not Process-Based**: Software organized as Cells, not processes
2. **Single Address Space**: All code shares one virtual address space
3. **Language-Based Isolation**: Rust's type system provides safety, not hardware MMU
4. **Zero-Copy IPC**: Ownership transfer instead of data copying
5. **Multi-Architecture**: Code must work on 32-bit and 64-bit systems

---

## The Golden Rules

### Rule 1: Interface is Sacred

**All changes to `libs/api` or `libs/types` require explicit user confirmation.**

These define the stable ABI between kernel and Cells. Breaking changes cascade throughout the system.

❌ **Don't**:
```rust
// libs/types/src/lib.rs
pub struct VAddr(pub usize);  // Changed from tuple struct to regular struct
```

✅ **Do**:
```rust
// Discuss breaking changes first
// Add new types alongside old ones during transition
pub type VAddrV1 = VAddr;  // Keep old API temporarily
pub struct VAddrV2 { addr: usize }  // New API
```

### Rule 2: Owned Buffers Only for Async

**Never pass `&mut [u8]` across async boundaries. Always use `Box<[u8]>`.**

❌ **Don't**:
```rust
async fn process_data(data: &mut [u8]) {
    some_async_call().await;
    data[0] = 42;  // DANGER: Lifetime violation possible
}
```

✅ **Do**:
```rust
async fn process_data(data: Box<[u8]>) -> Box<[u8]> {
    some_async_call().await;
    let mut data = data;  // Now safe to mutate
    data[0] = 42;
    data
}
```

**Why**: Rust's borrow checker cannot track borrows across await points in single address space.

### Rule 3: Multi-Architecture Awareness

**Never assume pointer size. Use types from `libs/types`.**

❌ **Don't**:
```rust
const KERNEL_BASE: u64 = 0xFFFF_FFFF_8000_0000;  // Breaks on 32-bit
let ptr = addr as u64;  // Breaks on 32-bit
```

✅ **Do**:
```rust
use types::{VAddr, PAddr};

const KERNEL_BASE: VAddr = VAddr(0x8000_0000);  // Portable
let ptr = VAddr::from(addr);  // Type-safe and portable
```

### Rule 4: Unsafe Management

**Cells**: `#![forbid(unsafe_code)]` - No unsafe allowed.
**Kernel/HAL**: `unsafe` only for hardware I/O, with `// SAFETY:` documentation.

❌ **Don't** (in Cell):
```rust
// cells/apps/shell/src/main.rs
unsafe {
    let ptr = 0x8000_0000 as *mut u8;
    *ptr = 0;  // FORBIDDEN in Cells
}
```

✅ **Do** (in Kernel):
```rust
// kernel/src/memory/paging.rs
unsafe {
    // SAFETY: SATP register write is only safe when:
    // 1. Page table is valid and properly initialized
    // 2. Identity mapping ensures kernel code remains accessible
    // 3. Called with interrupts disabled
    riscv::register::satp::write(root_table_ppn | satp_mode);
}
```

### Rule 5: Modern Module Style

**ABSOLUTELY FORBIDDEN: `mod.rs` files.**

Use modern Rust style: `foo.rs` parallel to `foo/` directory.

❌ **Don't**:
```
task/
├── mod.rs          # FORBIDDEN
├── tcb.rs
└── scheduler.rs
```

✅ **Do**:
```
task.rs             # Main task module
task/
├── tcb.rs
└── scheduler.rs
```

### Rule 6: ViOS Naming Convention

All public APIs use the `Vi` prefix to avoid naming conflicts with forked code.

| Type | Prefix/Rule | Example |
|------|-------------|---------|
| **Public Trait (ABI)** | `Vi` + PascalCase | `ViFileSystem`, `ViFile`, `ViDriver` |
| **Core Types/Errors** | `Vi` + PascalCase | `ViResult`, `ViError`, `ViConfig` |
| **Filesystem Implementations** | `vi` + Name + Version | `viFS1` (RedoxFS), `viFS2` (TFS) |
| **Address Types** | PascalCase | `VAddr`, `PAddr` |
| **Internal Modules** | snake_case | `task`, `memory`, `loader` |

### Rule 7: Trait Objects for Polymorphism

**Use `dyn Trait` for dynamic dispatch at system boundaries.**

✅ **Do**:
```rust
// For cells that can be loaded/unloaded dynamically
let fs: Arc<dyn ViFileSystem + Send + Sync> = ...;
let driver: Box<dyn ViDriver + Send + Sync> = ...;

// Thread-safe shared resources
static FS: Spinlock<Option<Arc<dyn ViFileSystem + Send + Sync>>> = ...;
```

**Why**: Cells are loaded dynamically, so we need runtime polymorphism.

### Rule 8: Resource Management & Drop

**Implement `Drop` for all resources that need cleanup.**

```rust
pub struct FileHandle {
    file: Box<dyn ViFile + Send + Sync>,
}

impl Drop for FileHandle {
    fn drop(&mut self) {
        // Cleanup happens here
        // Future: Could notify global file handle registry
        log::trace!("Closing file");
    }
}
```

**Why**: In SAS, there's no process cleanup. Resources must clean up explicitly.

---

## Project Structure

### Directory Layout

```
vios/
├── kernel/                    # Nano Kernel (Runtime Linker & Manager)
│   └── src/
│       ├── boot/              # Bootloader handoff
│       ├── cell/              # Cell metadata & lifecycle
│       ├── loader/            # ELF linker & relocator
│       ├── memory/            # Frame allocator & paging
│       ├── task/              # Task scheduler & TCB
│       ├── fs/                # Filesystem abstraction
│       └── main.rs            # Kernel entry point
│
├── hal/                       # Hardware Abstraction Layer
│   ├── core/                  # Facade (re-exports)
│   ├── traits/                # Pure trait definitions
│   │   ├── arch/              # Architecture trait
│   │   ├── uart/              # UART trait
│   │   ├── timer/             # Timer trait
│   │   ├── interrupt/         # Interrupt controller trait
│   │   └── display/           # Display trait
│   └── arch/                  # Implementation layer
│       ├── riscv/             # RISC-V implementation
│       │   └── src/
│       │       ├── common/    # RV32/RV64 shared code
│       │       ├── rv64/      # 64-bit specific
│       │       └── rv32/      # 32-bit specific
│       ├── arm/               # ARM implementation
│       └── x86/               # x86 implementation
│
├── libs/
│   ├── types/                 # Core types (VAddr, PAddr, ViError)
│   ├── api/                   # Trait definitions (ViFileSystem, etc.)
│   └── ostd/                  # Standard library for Cells
│
├── cells/
│   ├── apps/                  # User applications
│   │   ├── init/              # PID 1 equivalent
│   │   └── shell/             # Interactive shell
│   ├── drivers/               # Hardware drivers
│   │   ├── disk/
│   │   ├── gpu/
│   │   └── serial/
│   └── services/              # System services
│       ├── vfs/               # Virtual filesystem
│       ├── compositor/        # Display server
│       └── net/               # Network stack
│
└── tests/
    └── architecture-validation/
```

### Where to Add New Code

| What to Add | Where | Example |
|-------------|-------|---------|
| Core type | `libs/types/src/lib.rs` | `CellId`, `TaskId` |
| Public trait | `libs/api/src/*.rs` | `ViNetworkStack` trait |
| Kernel subsystem | `kernel/src/{name}/` | `kernel/src/ipc/` |
| HAL trait | `hal/traits/{name}/` | `hal/traits/spi/` |
| HAL implementation | `hal/arch/{arch}/src/` | `hal/arch/riscv/src/rv64/spi.rs` |
| Driver cell | `cells/drivers/{name}/` | `cells/drivers/spi/` |
| Service cell | `cells/services/{name}/` | `cells/services/audio/` |
| Application | `cells/apps/{name}/` | `cells/apps/editor/` |

---

## Naming Conventions

### File Naming

| Type | Pattern | Example |
|------|---------|---------|
| **Module file** | `snake_case.rs` | `task_scheduler.rs` |
| **Submodule directory** | `snake_case/` | `task/` |
| **Cell Cargo name** | `kebab-case` | `app-shell`, `drv-disk` |
| **Architecture variant** | `lowercase` | `rv64`, `aarch64`, `x86_64` |

### Rust Naming

| Item | Convention | Example |
|------|------------|---------|
| **Types** | PascalCase | `Task`, `FileHandle`, `ViError` |
| **Traits** | PascalCase with `Vi` prefix | `ViFileSystem`, `ViDriver` |
| **Functions** | snake_case | `init_memory()`, `schedule_next()` |
| **Variables** | snake_case | `frame_allocator`, `task_id` |
| **Constants** | SCREAMING_SNAKE_CASE | `PAGE_SIZE`, `MAX_TASKS` |
| **Statics** | SCREAMING_SNAKE_CASE | `FRAME_ALLOCATOR`, `SCHEDULER` |
| **Modules** | snake_case | `mod memory;` |

### Special Prefixes

**Public API Traits**: Start with `Vi`
```rust
pub trait ViFileSystem { }
pub trait ViDriver { }
pub trait ViBlockDevice { }
```

**Addresses**: Use `VAddr` / `PAddr`
```rust
pub struct VAddr(pub usize);  // Virtual address
pub struct PAddr(pub usize);  // Physical address
```

**Errors**: `ViError` prefix
```rust
pub enum ViError { ... }
pub type ViResult<T> = Result<T, ViError>;
```

---

## Code Organization

### Module Declaration (Modern Style)

**Parallel file/directory structure:**

```rust
// kernel/src/task.rs (main module)
pub mod tcb;       // task/tcb.rs
pub mod scheduler; // task/scheduler.rs

pub use tcb::Task;
pub use scheduler::Scheduler;

pub fn init() {
    // Task subsystem initialization
}
```

```rust
// kernel/src/task/tcb.rs
use crate::hal::arch::Context;

pub struct Task {
    pub id: usize,
    pub context: Context,
}
```

### Module Visibility

```rust
// Public API (visible outside crate)
pub fn public_function() { }

// Crate-internal (visible within kernel)
pub(crate) fn internal_function() { }

// Module-private (only in this file)
fn private_function() { }

// Super-visible (parent module)
pub(super) fn parent_visible() { }
```

### Re-exports

```rust
// kernel/src/lib.rs
pub use types::*;        // Re-export all types
pub use hal::Arch;       // Re-export HAL facade

// Makes these available as kernel::VAddr, kernel::Arch
```

---

## Writing Kernel Code

### Pattern: Global State with Spinlock

**All global state uses Spinlock with interrupt safety.**

```rust
use crate::sync::Spinlock;
use alloc::vec::Vec;

static SCHEDULER: Spinlock<Option<Scheduler>> = Spinlock::new(None);

pub fn schedule() {
    let mut sched = SCHEDULER.lock();  // Disables interrupts
    if let Some(scheduler) = sched.as_mut() {
        scheduler.schedule_next();
    }
    // Lock automatically released, interrupts restored
}
```

**Pattern Breakdown**:
1. `SCHEDULER.lock()` saves interrupt state and disables interrupts
2. Do work with exclusive access
3. On drop, lock releases and interrupt state restored

### Pattern: Frame Allocation

```rust
use crate::memory::frame::FRAME_ALLOCATOR;

pub fn alloc_page() -> Option<PAddr> {
    let mut allocator = FRAME_ALLOCATOR.lock();
    allocator.as_mut()?.allocate()
}

pub fn free_page(addr: PAddr) {
    let mut allocator = FRAME_ALLOCATOR.lock();
    if let Some(alloc) = allocator.as_mut() {
        alloc.deallocate(addr);
    }
}
```

### Pattern: Task Creation

```rust
// kernel/src/task/mod.rs
use crate::task::tcb::Task;
use crate::task::scheduler::SCHEDULER;

pub fn spawn(cell_id: CellId, entry_point: VAddr) -> Result<usize, ViError> {
    // 1. Create Task Control Block
    let task = Task::new(cell_id, entry_point)?;
    let task_id = task.id;

    // 2. Add to scheduler
    let mut sched = SCHEDULER.lock();
    let scheduler = sched.as_mut().ok_or(ViError::NotInitialized)?;
    scheduler.add_task(task);

    Ok(task_id)
}
```

### Pattern: Syscall Handler

```rust
// kernel/src/task/syscall.rs
use api::syscall::ViSyscall;

pub fn handle_syscall(syscall_id: usize, args: &[usize]) -> isize {
    let syscall = ViSyscall::from(syscall_id);

    match syscall {
        ViSyscall::Open => sys_open(args),
        ViSyscall::Read => sys_read(args),
        ViSyscall::Write => sys_write(args),
        ViSyscall::Close => sys_close(args),
        ViSyscall::Exit => sys_exit(args),
        _ => {
            log::error!("Unknown syscall: {}", syscall_id);
            -1  // Error
        }
    }
}

fn sys_open(args: &[usize]) -> isize {
    // args[0] = path pointer, args[1] = path length, args[2] = mode
    let path_ptr = args[0] as *const u8;
    let path_len = args[1];

    // SAFETY: Pointer must be valid from user task
    let path = unsafe {
        core::slice::from_raw_parts(path_ptr, path_len)
    };

    match open_file(path, args[2]) {
        Ok(fd) => fd as isize,
        Err(e) => -(e as isize),
    }
}
```

---

## Writing Cell Code

### Cell Structure

```rust
// cells/apps/myapp/src/main.rs
#![no_std]
#![no_main]
#![forbid(unsafe_code)]  // REQUIRED for Cells

extern crate alloc;
use ostd::prelude::*;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    ostd::init();  // Initialize Cell runtime

    match main() {
        Ok(_) => ostd::exit(0),
        Err(e) => {
            ostd::println!("Error: {:?}", e);
            ostd::exit(1);
        }
    }
}

fn main() -> ViResult<()> {
    ostd::println!("Hello from Cell!");

    // Use filesystem
    let mut file = ostd::fs::open("/test.txt", OpenMode::Read)?;
    let mut buffer = [0u8; 256];
    let n = file.read(&mut buffer)?;

    ostd::println!("Read {} bytes", n);
    Ok(())
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    ostd::println!("PANIC: {}", info);
    ostd::exit(1)
}
```

### Cell Cargo.toml

```toml
[package]
name = "app-myapp"
version = "0.1.0"
edition = "2021"

[dependencies]
ostd = { path = "../../../libs/ostd" }
types = { path = "../../../libs/types" }
api = { path = "../../../libs/api" }

[profile.dev]
panic = "abort"

[profile.release]
panic = "abort"
opt-level = "z"
lto = true
```

### Using Syscalls in Cells

```rust
use ostd::syscall;

fn send_message(target: usize, msg: &str) -> ViResult<()> {
    let msg_bytes = msg.as_bytes().to_vec().into_boxed_slice();
    syscall::send(target, msg_bytes, 0)?;
    Ok(())
}

fn receive_message() -> ViResult<String> {
    let (sender_id, msg_bytes) = syscall::recv(4096, 0)?;
    let msg = String::from_utf8_lossy(&msg_bytes).to_string();
    Ok(msg)
}
```

---

## Memory Management Patterns

### Pattern: Using Box for Ownership Transfer

```rust
// Allocate buffer
let buffer = vec![0u8; 4096].into_boxed_slice();

// Transfer ownership to another task via IPC
send_message(target_task, buffer);  // buffer moved, cannot access anymore
```

### Pattern: Using Arc for Shared Resources

```rust
use alloc::sync::Arc;
use crate::sync::Spinlock;

static FILESYSTEM: Spinlock<Option<Arc<dyn ViFileSystem + Send + Sync>>>
    = Spinlock::new(None);

pub fn register_filesystem(fs: Arc<dyn ViFileSystem + Send + Sync>) {
    let mut fs_lock = FILESYSTEM.lock();
    *fs_lock = Some(fs);
}

pub fn get_filesystem() -> Option<Arc<dyn ViFileSystem + Send + Sync>> {
    FILESYSTEM.lock().clone()
}
```

### Pattern: Stack Allocation

```rust
// For task stacks
pub fn allocate_stack() -> Result<VAddr, ViError> {
    const STACK_PAGES: usize = 16;  // 64KB
    let mut allocator = FRAME_ALLOCATOR.lock();

    // Allocate stack pages
    let stack_base = allocator.as_mut()
        .ok_or(ViError::NotInitialized)?
        .allocate_contiguous(STACK_PAGES)?;

    // Allocate guard page (for overflow detection)
    let guard_page = allocator.as_mut().unwrap()
        .allocate()
        .ok_or(ViError::OutOfMemory)?;

    Ok(stack_base)
}
```

### Pattern: Avoiding Memory Leaks

```rust
pub struct Lease {
    id: usize,
}

impl Drop for Lease {
    fn drop(&mut self) {
        // Revoke lease when dropped
        revoke_lease(self.id);
    }
}

// Usage
{
    let lease = create_lease(buffer);
    // Use lease...
}  // Lease automatically revoked here
```

---

## Error Handling

### Pattern: Result Propagation

```rust
fn open_and_read(path: &str) -> ViResult<Vec<u8>> {
    let mut file = fs::open(path, OpenMode::Read)?;  // Propagate error

    let mut buffer = Vec::new();
    loop {
        let mut chunk = [0u8; 4096];
        let n = file.read(&mut chunk)?;  // Propagate error
        if n == 0 { break; }
        buffer.extend_from_slice(&chunk[..n]);
    }

    Ok(buffer)
}
```

### Pattern: Error Context

```rust
fn load_config() -> ViResult<Config> {
    let data = fs::read_file("/etc/config.toml")
        .map_err(|_| ViError::ConfigLoadFailed)?;

    parse_config(&data)
        .map_err(|_| ViError::ConfigParseFailed)?
}
```

### Pattern: Logging Errors

```rust
fn try_operation() -> ViResult<()> {
    match risky_operation() {
        Ok(result) => {
            log::info!("Operation succeeded: {:?}", result);
            Ok(())
        }
        Err(e) => {
            log::error!("Operation failed: {:?}", e);
            Err(e)
        }
    }
}
```

---

## Async Programming

### Pattern: Async Trait Implementation

```rust
use async_trait::async_trait;

#[async_trait]
pub trait AsyncDriver: Send + Sync {
    async fn read(&mut self, buf: Box<[u8]>) -> ViResult<Box<[u8]>>;
    async fn write(&mut self, data: Box<[u8]>) -> ViResult<()>;
}

#[async_trait]
impl AsyncDriver for MyDriver {
    async fn read(&mut self, mut buf: Box<[u8]>) -> ViResult<Box<[u8]>> {
        // Perform async I/O
        self.wait_ready().await?;
        // Read data into buf
        Ok(buf)
    }

    async fn write(&mut self, data: Box<[u8]>) -> ViResult<()> {
        self.wait_ready().await?;
        // Write data
        Ok(())
    }
}
```

### Pattern: Owned Buffers for Async

```rust
// Correct: Owned buffer
async fn process(data: Box<[u8]>) -> ViResult<Box<[u8]>> {
    some_async_call().await?;
    // data still valid
    Ok(data)
}

// Incorrect: Borrowed buffer
// async fn process(data: &mut [u8]) -> ViResult<()> {
//     some_async_call().await?;  // ERROR: lifetime issues
//     Ok(())
// }
```

---

## Testing

### Unit Test Pattern

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_allocator() {
        let mut allocator = FrameAllocator::new(
            PAddr(0x8000_0000),
            PAddr(0x8800_0000)
        );

        // Allocate frame
        let frame1 = allocator.allocate().expect("Should allocate");
        assert!(frame1.0 >= 0x8000_0000);

        // Free frame
        allocator.deallocate(frame1);

        // Allocate again (should reuse)
        let frame2 = allocator.allocate().expect("Should allocate");
        assert_eq!(frame1, frame2);
    }
}
```

### Integration Test Pattern

```rust
// tests/architecture-validation/test_filesystem.rs
use types::*;
use api::fs::{ViFileSystem, OpenMode};

#[test]
fn test_file_operations() {
    let fs = create_test_filesystem();

    // Write file
    let mut file = fs.open("/test.txt", OpenMode::Write)
        .expect("Should create file");
    file.write(b"Hello, World!").expect("Should write");
    drop(file);

    // Read file
    let mut file = fs.open("/test.txt", OpenMode::Read)
        .expect("Should open file");
    let mut buffer = [0u8; 256];
    let n = file.read(&mut buffer).expect("Should read");

    assert_eq!(&buffer[..n], b"Hello, World!");
}
```

---

## Code Review Checklist

### Before Submitting PR

- [ ] Code follows ViOS naming conventions
- [ ] No `mod.rs` files used
- [ ] All `unsafe` blocks have `// SAFETY:` comments
- [ ] Cells have `#![forbid(unsafe_code)]`
- [ ] Multi-architecture compatible (no hardcoded pointer sizes)
- [ ] Async functions use `Box<[u8]>` not `&mut [u8]`
- [ ] Public traits use `Vi` prefix
- [ ] Error handling uses `ViResult<T>`
- [ ] Global state uses Spinlock
- [ ] Resources implement `Drop` for cleanup
- [ ] Code formatted with `cargo fmt`
- [ ] No clippy warnings (`cargo clippy`)
- [ ] Tests added for new functionality
- [ ] Documentation comments for public APIs
- [ ] Updated relevant `.codebase/*.md` if architecture changed

### Example Good PR Description

```markdown
## Summary
Add VirtIO SPI driver for peripheral communication

## Changes
- Added `hal/traits/spi/` with `ViSPI` trait
- Implemented in `hal/arch/riscv/src/common/spi.rs`
- Added driver cell in `cells/drivers/spi/`
- Updated documentation in `docs/DRIVERS.md`

## Testing
- [x] Tested in QEMU with virtual SPI device
- [x] Verified on both RV32 and RV64
- [x] All existing tests pass

## Checklist
- [x] No unsafe code in Cell
- [x] Public API uses `Vi` prefix
- [x] Formatted with cargo fmt
- [x] No clippy warnings
- [x] Documentation updated
```

---

## Common Patterns Reference

### Pattern: Initialization

```rust
pub fn init() {
    log::info!("Initializing subsystem");
    // Early setup, non-blocking
}
```

### Pattern: Singleton Access

```rust
static INSTANCE: Spinlock<Option<Instance>> = Spinlock::new(None);

pub fn get_instance() -> Option<Instance> {
    INSTANCE.lock().clone()
}
```

### Pattern: Iterator for Directory

```rust
pub struct DirIterator {
    file: Box<dyn ViFile + Send + Sync>,
}

impl Iterator for DirIterator {
    type Item = ViResult<DirEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.file.read_dir() {
            Ok(Some(entry)) => Some(Ok(entry)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}
```

---

## Quick Reference

### Must Read Before Coding

1. **`CLAUDE.md`** - Quick rules (auto-loaded each session)
2. **`docs/agent.md`** - The Constitution (complete rules)
3. **`docs/01-core.md`** - Cellular & Linker philosophy
4. **`docs/02-memory.md`** - SAS Layout & Memory rules
5. **`docs/03-runtime.md`** - Async Safety & Owned Buffers
6. **Relevant `docs/*.md`** for your feature area

### Common Commands

```bash
# Check code compiles
cargo check

# Format code
cargo fmt --all

# Run linter
cargo clippy -- -D warnings

# Build kernel
cargo build --release

# Run tests
cargo test --workspace

# Build specific cell
cargo build -p app-shell
```

---

**Last Updated**: 2026-01-07
**Version**: 0.2.0
**Maintainer**: ViOS Team
