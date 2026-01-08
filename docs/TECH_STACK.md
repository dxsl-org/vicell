# ViOS Technology Stack

> Complete overview of technologies, tools, libraries, and dependencies used in ViOS

## Table of Contents

1. [Overview](#overview)
2. [Core Technologies](#core-technologies)
3. [Development Tools](#development-tools)
4. [Runtime Dependencies](#runtime-dependencies)
5. [Build System](#build-system)
6. [Target Platforms](#target-platforms)
7. [Third-Party Libraries](#third-party-libraries)
8. [Architecture-Specific Tools](#architecture-specific-tools)

---

## Overview

ViOS is built using modern Rust and focuses on safety, performance, and portability across multiple architectures.

### Technology Philosophy

- **Safety First**: Memory-safe Rust prevents entire classes of bugs
- **Zero-Cost Abstractions**: High-level code compiles to efficient machine code
- **Multi-Architecture**: Support RISC-V, ARM, x86 from day one
- **no_std Environment**: Bare-metal kernel with no standard library dependencies

---

## Core Technologies

### Programming Language

| Technology | Version | Purpose | Rationale |
|------------|---------|---------|-----------|
| **Rust** | 2021 Edition (Nightly) | Primary language for kernel and cells | Memory safety, modern tooling, zero-cost abstractions |
| **Assembly** | RISC-V/ARM/x86 | Boot code and low-level operations | Required for CPU-specific initialization |

**Why Rust Nightly?**
- `#![feature(alloc_error_handler)]` - Custom OOM handler
- `#![feature(naked_functions)]` - Assembly function wrappers
- Future features for advanced type system usage

**Rust Features Used**:
```rust
#![no_std]                    // No standard library (bare metal)
#![no_main]                   // Custom entry point
#![feature(alloc_error_handler)]
#![feature(naked_functions)]
```

---

## Development Tools

### Toolchain

| Tool | Version | Purpose |
|------|---------|---------|
| **rustc** | nightly (2024-01+) | Rust compiler |
| **cargo** | 1.75+ | Build system and package manager |
| **rustfmt** | latest | Code formatting |
| **clippy** | latest | Linting and code quality |
| **rust-analyzer** | latest | IDE support (VS Code, IntelliJ) |

### Installation

```bash
# Install Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install nightly
rustup toolchain install nightly
rustup default nightly

# Add components
rustup component add rust-src rustfmt clippy

# Add target support
rustup target add riscv64gc-unknown-none-elf
rustup target add aarch64-unknown-none
rustup target add x86_64-unknown-none
```

### IDEs and Editors

**Recommended**: Visual Studio Code
- Extensions:
  - `rust-analyzer` - Language server
  - `CodeLLDB` - Debugging
  - `crates` - Dependency management
  - `Even Better TOML` - Cargo.toml editing

**Alternatives**:
- IntelliJ IDEA with Rust plugin
- Vim/Neovim with rust.vim + coc-rust-analyzer
- Emacs with rust-mode + lsp-mode

---

## Runtime Dependencies

### Kernel Dependencies

From `kernel/Cargo.toml`:

```toml
[dependencies]
types = { path = "../libs/types" }                # Core types (PhysAddr, VAddr, etc.)
api = { path = "../libs/api" }                     # Trait definitions
log = "0.4"                                        # Logging facade
virtio-drivers = "0.7.0"                          # VirtIO device drivers
linked_list_allocator = "0.10"                     # Heap allocator
hal = { path = "../hal/core" }                     # Hardware abstraction
async-trait = "0.1.89"                            # Async trait support
fatfs = { git = "..." }                            # FAT32 filesystem
xmas-elf = "0.9"                                  # ELF parser
riscv = "0.16.0"                                  # RISC-V CSR access
```

### Library Descriptions

#### Core ViOS Libraries

| Library | Location | Purpose |
|---------|----------|---------|
| **types** | `libs/types` | Fundamental types: `PhysAddr`, `VAddr`, `CellId`, `ViError` |
| **api** | `libs/api` | Trait definitions: `ViFileSystem`, `ViDriver`, `ViBlockDevice` |
| **ostd** | `libs/ostd` | Standard library replacement for Cells |
| **hal** | `hal/core` | Hardware abstraction layer facade |

#### Third-Party Crates

##### log (0.4)
- **Purpose**: Logging facade
- **Usage**: `log::info!()`, `log::warn!()`, `log::error!()`
- **Why**: Standard Rust logging interface
- **License**: MIT/Apache-2.0

##### virtio-drivers (0.7.0)
- **Purpose**: VirtIO device drivers (GPU, block, input, network)
- **Supported Devices**:
  - VirtIO Block (disk)
  - VirtIO GPU (graphics)
  - VirtIO Input (keyboard, mouse)
  - VirtIO Network (NIC)
- **Why**: Industry-standard paravirtualized devices
- **License**: MIT

##### linked_list_allocator (0.10)
- **Purpose**: Global heap allocator
- **Algorithm**: Linked list of free blocks (simple, suitable for kernel)
- **Performance**: O(n) allocation (acceptable for kernel heap)
- **Why**: no_std compatible, simple implementation
- **License**: Apache-2.0

##### xmas-elf (0.9)
- **Purpose**: ELF file parsing
- **Usage**: Load Cells dynamically at runtime
- **Features**: 32-bit and 64-bit ELF, relocations
- **Why**: Pure Rust, no_std compatible
- **License**: MIT/Apache-2.0

##### fatfs (git)
- **Purpose**: FAT32 filesystem implementation
- **Features**: Read/write, long filenames, no_std
- **Source**: https://github.com/rafalh/rust-fatfs
- **Why**: Simple, widely supported filesystem
- **License**: MIT

##### riscv (0.16.0)
- **Purpose**: RISC-V register access (CSRs, SATP, etc.)
- **Usage**: Configure paging, interrupts, timers
- **Why**: Standard crate for RISC-V development
- **License**: MIT/Apache-2.0

##### async-trait (0.1.89)
- **Purpose**: Enable async functions in traits
- **Usage**: Async drivers, services
- **Why**: Required for async trait methods
- **License**: MIT/Apache-2.0

---

## Build System

### Cargo Workspace

**Structure**:
```toml
[workspace]
members = [
    "kernel",
    "libs/*",
    "hal/core",
    "hal/traits/*",
    "hal/arch/*",
    "cells/drivers/*",
    "cells/services/*",
    "cells/apps/*",
]
```

**Benefits**:
- Shared dependency resolution
- Unified build commands
- Cross-crate refactoring support

### Build Profiles

```toml
[profile.dev]
panic = "abort"              # No unwinding (no_std requirement)

[profile.release]
panic = "abort"
lto = true                   # Link-time optimization
opt-level = "z"              # Optimize for size (embedded target)
```

**Why `panic = "abort"`?**
- Unwinding requires runtime support (incompatible with no_std)
- Reduces binary size significantly
- Simpler error handling model

**Why `opt-level = "z"`?**
- Minimizes kernel size
- Important for embedded deployments
- Tradeoff: Slightly slower than "3" but much smaller

### Build Commands

```bash
# Build kernel for RISC-V 64-bit
cargo build --release --target riscv64gc-unknown-none-elf

# Build all workspace members
cargo build --workspace

# Check without building (faster)
cargo check

# Run clippy linter
cargo clippy -- -D warnings

# Format code
cargo fmt --all

# Clean build artifacts
cargo clean
```

---

## Target Platforms

### Supported Architectures

| Architecture | Status | Target Triple | Features |
|--------------|--------|---------------|----------|
| **RISC-V 64-bit** | ✅ Primary | `riscv64gc-unknown-none-elf` | G=IMAFD, C=Compressed |
| **RISC-V 32-bit** | 🚧 WIP | `riscv32gc-unknown-none-elf` | For embedded devices |
| **ARM 64-bit** | 🚧 WIP | `aarch64-unknown-none` | Cortex-A series |
| **ARM 32-bit** | 🚧 WIP | `armv7a-none-eabi` | Cortex-A series |
| **x86_64** | 🚧 WIP | `x86_64-unknown-none` | Long mode |

### RISC-V Extensions

**Supported**:
- **I**: Integer base
- **M**: Multiplication/division
- **A**: Atomics (for spinlocks)
- **F**: Single-precision floating point
- **D**: Double-precision floating point
- **C**: Compressed instructions (16-bit)

**Target String**: `riscv64gc`
- `rv64` = 64-bit
- `g` = IMAFD (General purpose)
- `c` = Compressed

### Bootloaders

| Bootloader | Architecture | Status | Purpose |
|------------|--------------|--------|---------|
| **Limine** | RISC-V, x86, ARM | ✅ Primary | Modern, feature-rich bootloader |
| **OpenSBI** | RISC-V only | ✅ Fallback | Standard RISC-V firmware |
| **U-Boot** | All | 📅 Planned | Embedded systems |

---

## Third-Party Libraries

### Dependencies by Category

#### Memory Management
- `linked_list_allocator` - Heap allocator

#### File Systems
- `fatfs` - FAT32 implementation

#### Parsing
- `xmas-elf` - ELF parser
- `fdt` - Device Tree parser (for OpenSBI boot)

#### Hardware Drivers
- `virtio-drivers` - VirtIO devices

#### Utilities
- `log` - Logging facade
- `async-trait` - Async traits
- `riscv` - RISC-V CSR access

#### Data Structures
- `alloc::collections::BTreeMap` - Sorted map (from Rust core)
- `alloc::vec::Vec` - Dynamic array
- `alloc::boxed::Box` - Heap allocation
- `alloc::sync::Arc` - Atomic reference counting

---

## Architecture-Specific Tools

### RISC-V Tools

**Emulator**:
```bash
# QEMU RISC-V
qemu-system-riscv64 \
    -machine virt \
    -cpu rv64 \
    -smp 1 \
    -m 128M \
    -nographic \
    -bios default \  # OpenSBI
    -kernel kernel.elf
```

**Debugger**:
```bash
# GDB for RISC-V
riscv64-unknown-elf-gdb kernel.elf

# Commands
(gdb) target remote localhost:1234  # Connect to QEMU
(gdb) break kmain                   # Set breakpoint
(gdb) continue
(gdb) backtrace
```

**Disassembler**:
```bash
# Objdump
riscv64-unknown-elf-objdump -d kernel.elf > kernel.asm

# Readelf
riscv64-unknown-elf-readelf -a kernel.elf
```

### ARM Tools

**Emulator**:
```bash
# QEMU ARM
qemu-system-aarch64 \
    -machine virt \
    -cpu cortex-a57 \
    -m 128M \
    -nographic \
    -kernel kernel.elf
```

**Debugger**:
```bash
aarch64-none-elf-gdb kernel.elf
```

### x86 Tools

**Emulator**:
```bash
# QEMU x86
qemu-system-x86_64 \
    -machine q35 \
    -cpu max \
    -m 128M \
    -nographic \
    -kernel kernel.elf
```

**Debugger**:
```bash
gdb kernel.elf
```

---

## Testing Infrastructure

### Unit Testing

**Framework**: Rust's built-in test framework (limited in no_std)

```bash
# Run tests (for libs only, kernel cannot be tested traditionally)
cargo test --lib -p types
cargo test --lib -p api
```

### Integration Testing

**Location**: `tests/architecture-validation/`

**Test Files**:
- `step1_contract_testing.rs` - API contract verification
- `coverage_100_vmtrap.rs` - Virtual memory and trap handling
- `coverage_100_async.rs` - Async runtime testing
- `coverage_100_realtime.rs` - Real-time constraints

**Run**:
```bash
cargo test --test step1_contract_testing
```

### QEMU Testing

**Automated Boot Test**:
```bash
#!/bin/bash
timeout 5s qemu-system-riscv64 \
    -machine virt \
    -cpu rv64 \
    -nographic \
    -kernel kernel.elf \
    -serial stdio | tee boot.log

# Check for success message
grep "Kernel started" boot.log && echo "Boot test passed"
```

---

## Continuous Integration

### GitHub Actions (Planned)

```yaml
name: Build and Test

on: [push, pull_request]

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: nightly
          components: rust-src, rustfmt, clippy

      - name: Build kernel (RISC-V)
        run: cargo build --release --target riscv64gc-unknown-none-elf

      - name: Run clippy
        run: cargo clippy --all -- -D warnings

      - name: Check formatting
        run: cargo fmt --all -- --check

      - name: Run tests
        run: cargo test --workspace
```

---

## Dependency Security

### Audit Tools

```bash
# Install cargo-audit
cargo install cargo-audit

# Check for vulnerabilities
cargo audit

# Update dependencies
cargo update
```

### Supply Chain Security

**Strategy**:
- Pin exact versions in `Cargo.lock` (committed to git)
- Review dependency changes in PRs
- Prefer well-maintained crates with active communities
- Fork critical dependencies if needed (e.g., fatfs)

---

## Documentation Tools

### rustdoc

```bash
# Generate documentation
cargo doc --no-deps --open

# Document private items
cargo doc --document-private-items
```

### Mermaid Diagrams

**Usage**: Architecture diagrams in markdown

**Tools**:
- Mermaid Live Editor: https://mermaid.live
- VS Code extension: `bierner.markdown-mermaid`

---

## Performance Profiling

### Tools (Future)

| Tool | Purpose |
|------|---------|
| **cargo-flamegraph** | CPU profiling with flame graphs |
| **perf** | Linux perf for detailed analysis |
| **QEMU tracing** | Instruction-level tracing |

**Example**:
```bash
# Flamegraph (requires Linux perf)
cargo flamegraph --root

# QEMU tracing
qemu-system-riscv64 -d exec,int -D qemu.log ...
```

---

## Development Workflow

### Recommended Tools Stack

```
┌─────────────────────────────────────┐
│     IDE: VS Code + rust-analyzer    │
├─────────────────────────────────────┤
│     Build: Cargo + Rustc Nightly    │
├─────────────────────────────────────┤
│     Emulation: QEMU                 │
├─────────────────────────────────────┤
│     Debug: GDB / LLDB               │
├─────────────────────────────────────┤
│     Version Control: Git            │
└─────────────────────────────────────┘
```

### Daily Workflow

1. **Code**: Write Rust in VS Code with rust-analyzer
2. **Format**: `cargo fmt --all` (or on save)
3. **Check**: `cargo check` (fast iteration)
4. **Lint**: `cargo clippy` (catch common mistakes)
5. **Build**: `cargo build --release`
6. **Test**: Run in QEMU, check logs
7. **Debug**: GDB if issues arise
8. **Commit**: Git with descriptive messages

---

## Binary Size Optimization

### Techniques Used

1. **LTO (Link-Time Optimization)**:
   ```toml
   [profile.release]
   lto = true
   ```

2. **Size Optimization**:
   ```toml
   [profile.release]
   opt-level = "z"
   ```

3. **Strip Symbols**:
   ```bash
   riscv64-unknown-elf-strip kernel.elf
   ```

4. **Feature Minimization**:
   - Only enable needed features in dependencies
   - Example: `fatfs = { ..., default-features = false }`

### Size Analysis

```bash
# Check binary size
ls -lh target/riscv64gc-unknown-none-elf/release/vios-kernel

# Breakdown by section
riscv64-unknown-elf-size -A kernel.elf

# Identify large symbols
riscv64-unknown-elf-nm -S --size-sort kernel.elf | tail -20
```

---

## Future Technology Additions

### Planned Integrations

| Technology | Purpose | Status |
|------------|---------|--------|
| **smoltcp** | Network stack | 📅 Planned |
| **wasmi** | WASM interpreter | 📅 Planned |
| **wasmtime** | WASM JIT compiler | 📅 Future |
| **async-executor** | Async task executor | 🚧 In Progress |
| **embedded-graphics** | Display rendering | 📅 Planned |

---

## License Information

### Kernel License

**MPL 2.0** (Mozilla Public License 2.0)
- File-level copyleft
- Modifications to kernel files must be open-sourced
- Linking with proprietary Cells is allowed

### Third-Party Licenses

| Library | License |
|---------|---------|
| virtio-drivers | MIT |
| linked_list_allocator | Apache-2.0 |
| xmas-elf | MIT/Apache-2.0 |
| fatfs | MIT |
| riscv | MIT/Apache-2.0 |
| log | MIT/Apache-2.0 |

**Compliance**: All dependencies compatible with MPL 2.0

---

## References

### Official Documentation

- **Rust**: https://doc.rust-lang.org/
- **Cargo**: https://doc.rust-lang.org/cargo/
- **RISC-V**: https://riscv.org/specifications/
- **VirtIO**: https://docs.oasis-open.org/virtio/

### Learning Resources

- **Rust Embedded Book**: https://rust-embedded.github.io/book/
- **RISC-V Assembly**: https://riscv.org/wp-content/uploads/2017/05/riscv-spec-v2.2.pdf
- **OSDev Wiki**: https://wiki.osdev.org/

---

**Last Updated**: 2026-01-07
**Version**: 0.2.0
**Maintainer**: ViOS Team
