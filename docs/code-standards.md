# ViOS Code Standards

**Scope**: Rust code across kernel, HAL, libraries, and Cells  
**Edition**: 2021  
**Nightly**: Required for `no_std` bare-metal features  
**Last Updated**: 2026-06-03

---

## The 8 Coding Laws (Non-Negotiable)

### Law 1: Interface is Sacred

- **Scope**: `libs/api/` and `libs/types/`
- **Rule**: Any changes require 2x explicit user confirmation
- **Reason**: These define the stable ABI between kernel and Cells
- **Implementation**:
  - Use `#[repr(C)]` for all public traits to ensure C compatibility
  - Document trait contract in doc comments
  - Preserve method signatures when extending traits

### Law 2: Owned Buffers for Async (SAS Safety)

**Forbidden**:
```rust
async fn process(data: &mut [u8]) { }  // ❌ LIFETIME VIOLATION
```

**Required**:
```rust
async fn process(data: Box<[u8]>) -> Box<[u8]> { }  // ✅ OWNED
```

**Why**: Single Address Space (SAS) means no process boundaries for cleanup. Owned buffers ensure deterministic drop semantics across async boundaries.

**Pattern**:
- Input: `Box<[u8]>` or `Vec<u8>` (caller owns until call)
- Output: `Box<[u8]>` or `Vec<u8>` (callee owns return)
- Channels: `mpsc::Sender<Box<[u8]>>` for zero-copy IPC

### Law 3: Multi-Architecture Awareness

**Forbidden**:
```rust
let addr: u64 = 0xFFFF_FFFF_8000_0000;  // ❌ ASSUMES 64-BIT
```

**Required**:
```rust
let addr = VAddr(0x8000_0000);  // ✅ ARCH-AGNOSTIC
```

**Rules**:
- Never hardcode pointer sizes (`usize`, `u64`)
- Always use `VAddr` for virtual addresses, `PAddr` for physical
- Test on RV32, RV64, and ARM targets (compile checks at minimum)

### Law 4: Unsafe Code Management

**Cells**:
```rust
#![forbid(unsafe_code)]  // ABSOLUTE
```

**Kernel & HAL**:
- Unsafe only for hardware I/O (CSRs, MMIO)
- **Every `unsafe` block must have a `// SAFETY:` comment** explaining:
  - Why safety invariants are maintained
  - What preconditions the caller must satisfy
  - What could go wrong if misused

**Example**:
```rust
// SAFETY: We assume mmu is initialized and this vaddr is mapped in current page table.
// CSR access is safe: no concurrent hart touches mepc during boot.
unsafe { riscv::register::mepc::write(func as usize); }
```

### Law 5: Modern Module Structure

**Forbidden**:
```
foo/
├── mod.rs      ❌ BANNED
└── bar.rs
```

**Required**:
```
foo.rs          ✅ REQUIRED (parallel file + folder)
foo/
├── bar.rs
└── baz.rs
```

**Rules**:
- Declare module in parent: `mod foo;`
- Module file: `foo.rs` (re-exports what's in `foo/` folder)
- Submodules: `foo/bar.rs`, `foo/baz.rs`
- Use snake_case: `file_system.rs`, not `FileSystem.rs`

**Why**: Clearer file tree, easier IDE navigation, prevents accidental circular imports.

### Law 6: ViOS Naming Convention

| Category | Rule | Examples |
|----------|------|----------|
| **Public Traits** | `Vi` prefix (Vi-something) | `ViFileSystem`, `ViDriver`, `ViBlockDevice`, `ViNetTcpStack` |
| **Error Types** | `Vi` prefix | `ViError`, `ViResult<T>` |
| **Core Structs** | `Vi` prefix (or generic) | `ViConfig`, `ViBenchmark` |
| **Address Types** | `V` or `P` prefix | `VAddr`, `PAddr` |
| **Filesystem Names** | `vi` lowercase | `viFS1` (RedoxFS), `viFS2` (TFS) |
| **Modules/Files** | snake_case | `task.rs`, `memory.rs`, `frame_allocator.rs` |
| **Functions** | snake_case | `init_paging`, `handle_interrupt` |
| **Constants** | UPPER_SNAKE | `MAX_CELLS`, `KERNEL_HEAP_SIZE` |
| **Type Params** | PascalCase | `T`, `E`, `CellState` |

### Law 7: Trait Objects for Polymorphism

**Pattern**:
```rust
pub fn register_driver(driver: Arc<dyn ViDriver + Send + Sync>) { }
```

**Rules**:
- Use `dyn Trait` at system boundaries (Cells, drivers, services)
- Always specify bounds: `Send + Sync` for multi-cell safety
- `Box<dyn T>` for single owner (Cell)
- `Arc<dyn T>` for shared resources (kernel registry)
- Implement `Drop` for cleanup (Law 8)

**Why**: Enables dynamic Cell loading without recompilation.

### Law 8: RAII - Implement Drop

**Rule**: All resources must implement `Drop` for explicit cleanup.

**Pattern**:
```rust
pub struct FileHandle { fd: u32 }

impl Drop for FileHandle {
    fn drop(&mut self) {
        // Close file, release resource
        syscall::close(self.fd).ok();
    }
}
```

**Why**: In SAS, there's no process cleanup. Resources don't auto-free when a task dies. You must manually manage.

**Resources Requiring Drop**:
- `FileHandle`, `DirHandle` — system resources
- `GrantEntry`, `Lease` — capability objects
- `Lock<T>` — mutual exclusion
- Custom allocations — via `alloc` crate

---

## Error Handling

### Result Pattern (Not Panic)

```rust
pub type ViResult<T> = Result<T, ViError>;
```

**Rule**: Use `Result<T, E>` everywhere except kernel invariants.

**ViError Variants**:
```rust
pub enum ViError {
    OutOfMemory,
    InvalidArgument,
    NotFound,
    PermissionDenied,
    AlreadyExists,
    WouldBlock,
    NotSupported,
    IO(String),
    InvalidInput,
    IsADirectory,
    NotADirectory,
    Unknown,
}
```

**Syscall Wrapper Example**:
```rust
pub fn open(path: &str, flags: u32) -> ViResult<FileHandle> {
    let fd = unsafe { syscall(SysCall::Open, path, flags)? };
    Ok(FileHandle { fd })
}
```

---

## Async & Concurrency

### Async Functions

```rust
pub async fn read_file(path: &str) -> ViResult<Vec<u8>> {
    let file = open(path, READ).await?;
    file.read_all().await
}
```

**Rules**:
- Use `async/await` syntax (not `Future` trait directly)
- Owned buffers: `Box<[u8]>`, never `&mut [u8]`
- Spawn tasks with kernel executor: `spawn_async(future)`

### Spinlocks for Synchronization

```rust
static REGISTRY: Spinlock<HashMap<CellId, Cell>> = Spinlock::new(HashMap::new());

fn register(id: CellId, cell: Cell) {
    let mut map = REGISTRY.lock();
    map.insert(id, cell);
}
```

**Why**: Spinlock handles interrupt safety automatically (disables on lock, re-enables on drop).

---

## Testing

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_allocation() {
        let alloc = FrameAllocator::new();
        let frame = alloc.allocate().expect("should allocate");
        assert!(frame.0 > 0);
    }
}
```

**Rules**:
- Test critical logic (allocators, scheduler, IPC)
- Use `expect()` with clear messages, not `unwrap()`
- No integration tests in kernel (use architecture tests in `tests/`)

### Integration Tests

Located in `tests/architecture-validation/`:
```
tests/
├── step1_spec_verification.md
├── step2_dependency_analysis.md
└── (20+ checks)
```

**Run**:
```bash
cargo test --test '*' --release
```

---

## Comments & Documentation

### Doc Comments (Public Items)

```rust
/// Opens a file from the virtual filesystem.
///
/// # Arguments
/// * `path` - Absolute path (e.g., "/bin/hello")
/// * `flags` - Open flags (READ, WRITE, APPEND)
///
/// # Returns
/// A `FileHandle` or `ViError` if not found or permission denied.
///
/// # Example
/// ```
/// let handle = open("/bin/hello", READ)?;
/// let bytes = handle.read_all().await?;
/// ```
pub async fn open(path: &str, flags: u32) -> ViResult<FileHandle> { }
```

**Rules**:
- Document all public traits, functions, types
- Include # Arguments, # Returns, # Errors sections
- Add examples for complex logic
- Link to related specs: `See docs/specs/03-runtime.md for async safety rules.`

### Safety Comments

```rust
// SAFETY: We guarantee UART is initialized before this point.
// CSR access is atomic: no other hart modifies mepc during boot.
unsafe { riscv::register::mepc::write(func as usize); }
```

**Format**:
```
// SAFETY: [Why it's safe: preconditions, guarantees, no data races]
unsafe { ... }
```

### Inline Comments (Sparse)

Only when WHAT the code does is unclear:

```rust
// Bad:
x = x + 1;  // Increment x

// Good:
// Align heap pointer to next 4KB boundary (page size)
heap_ptr = (heap_ptr + 0xFFF) & !0xFFF;
```

---

## Code Organization

### Imports

```rust
// System imports (std, no_std)
use core::ptr;

// External crates
use spin::Spinlock;
use xmas_elf::ElfFile;

// This crate
use crate::memory::{VAddr, PAddr};
use crate::task::Task;

// Pub re-exports at module level
pub use crate::types::{ViError, ViResult};
```

**Order**: System → External → Internal → Re-exports.

### File Size

- **Limit**: 200-300 LOC per file
- **Exceeding**: Split into submodules
- **Example**: `task.rs` (1000 LOC) → `task/scheduler.rs`, `task/syscall.rs`, `task/ipc.rs`

### Visibility

```rust
// Kernel only
fn internal_fn() { }

// Public to cells (part of syscall ABI)
pub unsafe fn syscall_handler() { }

// Public trait (stable ABI)
#[repr(C)]
pub trait ViFileSystem {
    fn open(&self, path: &str) -> ViResult<Box<dyn ViFile>>;
}
```

---

## Build & Compilation

### Cargo Features

```toml
[features]
default = ["riscv64"]
riscv32 = []
riscv64 = []  # Primary target
arm64 = []
x86_64 = []
```

**Conditional Code**:
```rust
#[cfg(target_arch = "riscv64")]
pub fn init_paging() { /* SV39 */ }

#[cfg(target_arch = "arm")]
pub fn init_paging() { /* ARMv8 */ }
```

### Compiler Flags

```toml
[profile.release]
panic = "abort"        # No unwinding in kernel
lto = true             # Whole program optimization
opt-level = "z"        # Size + speed tradeoff
```

---

## Common Patterns

### Global State (Kernel)

```rust
static SCHEDULER: Spinlock<RoundRobin> = Spinlock::new(RoundRobin::new());

pub fn schedule() {
    let mut sched = SCHEDULER.lock();
    sched.next_task();
}
```

### Capability Object (Syscall)

```rust
pub struct Grant {
    capability: Capability,
    from: CellId,
    to: CellId,
}

impl Drop for Grant {
    fn drop(&mut self) {
        // Revoke capability on drop
    }
}
```

### Async Executor Task

```rust
pub async fn read_with_timeout(path: &str, timeout_ms: u64) -> ViResult<Vec<u8>> {
    select! {
        result = read_file(path) => result,
        _ = timer::sleep(timeout_ms) => Err(ViError::WouldBlock),
    }
}
```

---

## Deprecations & Breaking Changes

When changing public API in `libs/api/`:

```rust
#[deprecated(since = "0.3.0", note = "use ViAsyncFileSystem instead")]
pub trait ViFileSystem {
    // old impl
}

pub trait ViAsyncFileSystem {
    // new impl
}
```

**Changelog Entry** (in `docs/project-changelog.md`):
```markdown
## [0.3.0] - 2026-06-15
### Deprecated
- `ViFileSystem::open()` → use `ViAsyncFileSystem::open().await` instead
```

---

## Quick Reference Card

| Rule | Status | Enforcement |
|------|--------|-------------|
| No mod.rs | ❌ FORBIDDEN | CI lint |
| Owned buffers in async | ❌ FORBIDDEN | Compiler error |
| Unsafe requires SAFETY comment | ❌ FORBIDDEN | Code review |
| Cells can't use unsafe | ❌ FORBIDDEN | `#![forbid(unsafe_code)]` |
| Vi prefix for public traits | ✅ REQUIRED | Code review |
| Result<T, E> over panic! | ✅ REQUIRED | Code review |
| Implement Drop | ✅ REQUIRED | Code review |
| 200-300 LOC per file | ✅ GUIDELINE | Code review |

---

## See Also

- **CLAUDE.md** — Quick agent reference
- **patterns.md** — Deep patterns & examples
- **system-architecture.md** — System design
- **api-reference.md** — Full trait reference
- Specs: **docs/specs/0X-*.md** — Feature specifications
