# ViOS AI Agent Guidelines

> **Auto-loaded context for Claude Code CLI - Essential rules for every session**

---

## 🔴 PRIME DIRECTIVE

**ViOS uses Cellular SAS (Single Address Space) + Language-Based Isolation (LBI)**

- ❌ **NOT** traditional Linux/Unix process-based thinking
- ✅ **YES** Cellular architecture with zero-copy IPC
- ✅ **YES** Rust type system for safety, not hardware MMU

**Key Philosophy**: Software organized as **Cells** (not processes), sharing one address space, isolated by Rust's type system.

---

## ⚡ The 8 Coding Laws (Must Follow)

### Law 1: Interface is Sacred
- Any changes to `libs/api/` or `libs/types/` require **2x user confirmation**
- These define the stable ABI between kernel and Cells
- Use `#[repr(C)]` for all public traits

### Law 2: Owned Buffers for Async (SAS Safety)
- ❌ **FORBIDDEN**: `async fn process(data: &mut [u8])`
- ✅ **REQUIRED**: `async fn process(data: Box<[u8]>) -> Box<[u8]>`
- **Why**: Prevent lifetime violations in Single Address Space

### Law 3: Multi-Architecture Awareness
- Never assume 32-bit or 64-bit
- Use `VAddr`, `PAddr` from `libs/types`
- ❌ Don't: `let addr: u64 = 0xFFFF_FFFF_8000_0000`
- ✅ Do: `let addr = VAddr(0x8000_0000)`

### Law 4: Unsafe Management
- **Cells**: `#![forbid(unsafe_code)]` - NO exceptions
- **Kernel/HAL**: `unsafe` only for hardware I/O, must document with `// SAFETY:`

### Law 5: Modern Module Style
- ❌ **ABSOLUTELY FORBIDDEN**: `mod.rs` files
- ✅ **REQUIRED**: `foo.rs` parallel to `foo/` directory
- Use snake_case for files/directories

### Law 6: ViOS Naming Convention
- **Public Traits**: `Vi` prefix → `ViFileSystem`, `ViDriver`, `ViBlockDevice`
- **Core Types**: `Vi` prefix → `ViError`, `ViResult`, `ViConfig`
- **Addresses**: `VAddr` (virtual), `PAddr` (physical)
- **Filesystems**: `viFS1` (RedoxFS), `viFS2` (TFS), etc.

### Law 7: Trait Objects for Polymorphism
- Use `dyn Trait` for system boundaries (enables dynamic Cell loading)
- Always specify bounds: `Arc<dyn ViDriver + Send + Sync>`
- `Box` for single owner, `Arc` for shared resources

### Law 8: RAII - Implement Drop
- All resources **must** implement `Drop` for cleanup
- No process cleanup in SAS - resources must clean up explicitly
- Example: `FileHandle`, `Lease`, `GrantEntry`

---

## 📚 Before Coding - Read Specifications

| Task Area | Read First |
|-----------|------------|
| **Fork code from other projects** | `design/00-fork.md` |
| **Cellular philosophy & Linker** | `design/01-core.md` |
| **Memory (SAS, HHDM, Registry)** | `design/02-memory.md` |
| **Async safety & Owned Buffers** | `design/03-runtime.md` |
| **Multi-arch HAL (RV32/64/128)** | `design/04-hardware.md` |
| **Native/WASM/VM applications** | `design/05-application.md` |
| **Graphics & Compositor** | `design/06-graphics.md` |
| **Network stack** | `design/07-networking.md` |
| **Power management** | `design/08-power.md` |
| **Filesystem (VFS)** | `design/09-vfs.md` |
| **Testing strategy** | `design/10-testing.md` |

---

## 🔧 Agent Workflow (Standard Process)

```
1. Check Specs → Read relevant design/*.md to understand "Why"
2. Interface First → Define traits in libs/api/ before implementation
3. Implement → Write code, use Result<T, E> not panic!
4. Verify → Write tests for critical logic
```

---

## 📁 Project Structure (Quick Reference)

```
vios/
├── kernel/src/
│   ├── boot/        # Bootloader handoff
│   ├── cell/        # Cell metadata & lifecycle
│   ├── loader/      # ELF linker & relocator
│   ├── memory/      # Frame allocator & paging
│   └── task/        # Task scheduler (not process!)
├── hal/
│   ├── core/        # Facade (re-exports)
│   ├── traits/      # Pure trait definitions
│   └── arch/        # riscv/, arm/, x86/
├── libs/
│   ├── types/       # VAddr, PAddr, ViError
│   ├── api/         # Trait definitions (ABI)
│   └── ostd/        # Std lib for Cells
└── cells/
    ├── apps/        # Applications
    ├── drivers/     # Hardware drivers
    └── services/    # System services
```

---

## 🎯 Critical Patterns (Quick Reminder)

### Global State Pattern
```rust
static INSTANCE: Spinlock<Option<T>> = Spinlock::new(None);
// Spinlock automatically handles interrupt safety
```

### Error Handling
```rust
pub type ViResult<T> = Result<T, ViError>;
// Use Result, not panic! (except for kernel invariants)
```

### Resource Cleanup
```rust
impl Drop for ResourceHandle {
    fn drop(&mut self) {
        // Cleanup happens here automatically
    }
}
```

---

## 📖 Complete Documentation

**Full Rules & Details**: [`design/00-context.md`](./design/00-context.md)

**For Developers**:
- [ARCHITECTURE.md](./docs/ARCHITECTURE.md) - System design
- [CODING_GUIDE.md](./docs/CODING_GUIDE.md) - How to write code
- [PATTERNS.md](./docs/PATTERNS.md) - Common patterns
- [API.md](./docs/API.md) - Complete API reference
- [ONBOARDING.md](./docs/ONBOARDING.md) - Getting started

---

## ⚠️ Common Mistakes to Avoid

❌ **Don't**:
- Use `mod.rs` files
- Pass `&mut [u8]` across async boundaries
- Assume pointer size (use VAddr/PAddr)
- Put unsafe code in Cells
- Change libs/api without confirmation
- Use traditional process-based thinking

✅ **Do**:
- Read specifications first
- Use owned buffers (Box/Arc)
- Follow naming conventions (Vi prefix)
- Implement Drop for resources
- Think in terms of Cells, not processes
- Document unsafe with // SAFETY:

---

## 🚀 Quick Command Reference

```bash
# Check code
cargo check

# Format
cargo fmt --all

# Lint
cargo clippy -- -D warnings

# Build
cargo build --release

# Run
./run.ps1  # or qemu command
```

---

**Version**: 0.2.0
**Last Updated**: 2026-01-07
**Full Rules**: See [`design/00-context.md`](./design/00-context.md) for complete specifications

---

*This file is automatically loaded by Claude Code CLI at the start of each session to provide essential context about ViOS coding standards and architecture.*
