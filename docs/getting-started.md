# Getting Started with ViOS

> Your complete guide to building and contributing to ViOS
>
> **Version**: 0.2.1-dev | **Last Updated**: 2026-06-03

## Quick Start

Get ViOS running in 5 steps. Expect 30–45 minutes on your first setup.

### Prerequisites Table

| Requirement | Version | Why |
|-------------|---------|-----|
| **Rust** | nightly | Compiler for no_std kernel |
| **QEMU** | 7.0+ | RISC-V emulator (replaces physical hardware) |
| **Python** | 3.8+ | Disk image creation script |
| **Git** | 2.30+ | Source control |
| **RAM** | 4GB min | Development environment needs breathing room |

### Setup Steps

```bash
# 1. Install Rust nightly + components
rustup default nightly
rustup component add rust-src rustfmt clippy
rustup target add riscv64gc-unknown-none-elf

# 2. Install QEMU
# Ubuntu: sudo apt install qemu-system-riscv64
# macOS: brew install qemu
# MSYS2: pacman -S mingw-w64-x86_64-qemu

# 3. Clone and enter repository
git clone https://github.com/your-org/vios.git
cd vios

# 4. Build kernel
cargo build --release

# 5. Create disk + run
python3 create_ramdisk.py
./run.ps1  # Windows PowerShell | Linux/macOS: ./scripts/run-riscv64.sh
```

### First Run Output

You'll see:
```
VIOS K MAIN ENTRY
[INFO] Kernel started (Hart: 0, DTB: 0x87000000)
[INFO] Frame allocator initialized
[INFO] Paging initialized
[INFO] Heap initialized
[INFO] Scheduler initialized
ViOS Shell v0.2.0
vios>
```

Type `help` to see available commands. Type `exit` to quit QEMU.

---

## Understanding ViOS

### What Makes ViOS Different?

ViOS is a **Cellular Single Address Space (SAS) Operating System** — revolutionary compared to traditional designs.

| Aspect | Traditional OS | ViOS |
|--------|---|---|
| **Isolation** | Hardware MMU + processes | Rust type system + Cells |
| **Address Space** | Separate per process | Single shared address space |
| **IPC** | Copy overhead | Zero-copy ownership transfer |
| **Kernel Size** | Monolithic (Linux: 7M lines) | Nano kernel (~7,000 lines) |

### Key Concepts

**Cells**: Independent compiled units (not processes). Each Cell can be Native Rust, WASM, or sandboxed C/C++. Loaded and linked at runtime.

**Single Address Space**: All Cells share one virtual address space. No address space switches, no context overhead. Direct pointer sharing within safety rules.

**Language-Based Isolation (LBI)**: Safety from Rust, not hardware. Cells compiled with `#![forbid(unsafe_code)]`. Borrow checker prevents memory bugs. Type system enforces contracts.

**Zero-Copy IPC**: Data transferred via ownership (like Rust `move`). Two mechanisms:
- **Lease**: Temporary borrow (like `&T` or `&mut T`)
- **Grant**: Permanent transfer (like Rust ownership move)

### Architecture Layers

```
Cells (Apps, Services, Drivers)  ← Your code runs here
Nano Kernel (7K LOC)             ← ELF loader, scheduler, memory, IPC
Hardware Abstraction Layer       ← RISC-V, ARM, x86 support
```

---

## Codebase Tour

### Directory Map

```
kernel/src/        Nano kernel core (boot, cell, loader, memory, task)
hal/               Hardware abstraction (traits/, arch/{riscv,arm,x86})
libs/              Core types (types/, api/) and Cell stdlib (ostd/)
cells/             Applications, services, drivers
docs/              All documentation including this file
```

### 20-Minute Reading List

1. **CLAUDE.md** (root) — 8 Coding Laws (3 min)
2. **system-architecture.md** → Overview section (5 min)
3. **code-standards.md** → The 8 Laws in detail (2 min)
4. **This file** (remaining 10 min)

### Follow a Syscall

Trace how code from a Cell reaches the kernel:

```rust
// Cell calls: cells/apps/shell/src/main.rs
let file = ostd::fs::open("/test.txt", Read)?;
    ↓
// Wrapper: libs/ostd/src/fs.rs
pub fn open(path: &str, mode: OpenMode) -> ViResult<Box<dyn ViFile>> {
    unsafe { syscall::syscall3(ViSyscall::Open as usize, ...) }
    ↓
// Trap handler: hal/arch/riscv/src/rv64/trap.rs
pub fn trap_handler(frame: &mut ViTrapFrame) {
    // Branch on syscall number
    ↓
// Kernel handler: kernel/src/task/syscall.rs
pub fn handle_syscall(id: usize, args: &[usize]) -> isize {
    match ViSyscall::from(id) {
        ViSyscall::Open => sys_open(args),
        ...
```

### Key Files

| File | Purpose |
|------|---------|
| `kernel/src/main.rs` | Kernel entry point (kmain) |
| `kernel/src/task/scheduler.rs` | Task scheduling logic |
| `kernel/src/memory/frame.rs` | Physical frame allocator |
| `kernel/src/loader/elf.rs` | ELF loader + runtime linker |
| `libs/types/src/lib.rs` | VAddr, PAddr, ViError, ViResult |
| `libs/api/src/*.rs` | Trait definitions (stable ABI) |
| `hal/arch/riscv/src/rv64/` | RISC-V 64-bit HAL impl |

---

## Building & Running

### Build Profiles

```bash
# Debug: fast compile, large binary, debug symbols
cargo build

# Release: slower compile, optimized binary (use this for QEMU)
cargo build --release

# Just check (no compile): fastest iteration
cargo check

# Format + lint before commit
cargo fmt --all && cargo clippy -- -D warnings
```

### Creating Disk Image

```bash
# Automated (recommended)
python3 create_ramdisk.py

# Manual (Linux/macOS)
dd if=/dev/zero of=disk.img bs=1M count=40
mkfs.vfat disk.img
```

### Running in QEMU

**Windows PowerShell**:
```powershell
.\run.ps1
```

**Linux/macOS** (manual):
```bash
qemu-system-riscv64 \
  -machine virt -cpu rv64 -smp 1 -m 128M \
  -nographic -bios default \
  -kernel target/riscv64gc-unknown-none-elf/release/vios-kernel \
  -drive file=disk.img,format=raw,if=none,id=hd0 \
  -device virtio-blk-device,drive=hd0
```

**Exit QEMU**: Press `Ctrl+A` then `X` (or `Ctrl+C` multiple times).

### QEMU Parameters Quick Reference

| Flag | Meaning |
|------|---------|
| `-machine virt` | Generic RISC-V virtual machine |
| `-cpu rv64,c=true` | RISC-V 64-bit with compression |
| `-m 128M` | RAM size |
| `-nographic` | Terminal-only (no GUI window) |
| `-bios default` | OpenSBI firmware |
| `-kernel` | Path to kernel binary |
| `-drive` / `-device` | Disk configuration |

---

## Debugging

### GDB Setup

```bash
# Terminal 1: Start QEMU in debug mode (pauses at boot)
qemu-system-riscv64 -machine virt -cpu rv64 -m 128M \
  -nographic -bios default \
  -kernel target/riscv64gc-unknown-none-elf/debug/vios-kernel \
  -drive file=disk.img,format=raw,if=none,id=hd0 \
  -device virtio-blk-device,drive=hd0 \
  -s -S

# Terminal 2: Connect GDB
riscv64-unknown-elf-gdb target/riscv64gc-unknown-none-elf/debug/vios-kernel
(gdb) target remote localhost:1234
(gdb) break kmain
(gdb) continue
```

### Common Errors & Fixes

| Error | Cause | Fix |
|-------|-------|-----|
| `can't find crate for 'core'` | Missing `rust-src` | `rustup component add rust-src` |
| `qemu-system-riscv64: command not found` | QEMU not installed | See prerequisites table above |
| `linker 'riscv64-unknown-elf-gcc' not found` | Missing cross-toolchain | `apt install gcc-riscv64-linux-gnu` |
| Kernel boots then hangs | Disk image missing | Run `python3 create_ramdisk.py` |
| `rustup: toolchain 'nightly-...' not installed` | Stale config | `rustup toolchain install nightly` |
| Black screen (no output) | Missing `-nographic` flag | Verify QEMU command includes `-nographic` |
| `can't find 'alloc' feature` | Missing feature in Cargo.toml | Add `features = ["alloc"]` to dep |

---

## Your First Contribution

### Good First Issues

**Easy** (1–2 hours): Add a shell command, fix typo, improve error messages, write tests.

**Medium** (1 day): Add new Cell app, implement shell command, improve logging.

**Challenging** (3–5 days): New HAL trait, filesystem, scheduler optimization, driver Cell.

### Example: Add `uptime` Shell Command

```bash
# 1. Create branch
git checkout -b feature/shell-uptime

# 2. Edit cells/apps/shell/src/main.rs
# Find command dispatch (match command { ... })
# Add:
"uptime" => {
    let ticks = ostd::syscall::get_system_ticks();
    ostd::println!("Uptime: {} seconds", ticks / 1000);
}

# 3. Build and test
cargo build --release && python3 create_ramdisk.py && ./run.ps1
# In ViOS: uptime

# 4. Commit (conventional format)
git commit -m "feat(shell): add uptime command

Shows system uptime in seconds."

# 5. Push + create PR
git push origin feature/shell-uptime
```

### PR Checklist

- [ ] Follows [code-standards.md](./code-standards.md) rules
- [ ] No unsafe in Cells (`#![forbid(unsafe_code)]`)
- [ ] Error handling via `Result<T, E>`
- [ ] Builds without warnings
- [ ] Tested in QEMU
- [ ] Conventional commit message

---

## Getting Help

### Where to Ask

- **GitHub Discussions**: Design questions, "how does X work?"
- **GitHub Issues**: Bug reports, feature requests
- **Discord/Matrix**: Real-time help, quick questions

### Question Template

```markdown
## What I'm trying to do
Add a new syscall for CPU frequency.

## What I've tried
1. Read system-architecture.md
2. Looked at existing syscalls
3. Added syscall number to enum

## Where I'm stuck
How do I get CPU frequency from HAL?

## Environment
ViOS: main (commit abc123) | Target: RISC-V 64 | Host: Linux
```

### Debug Tips

**Kernel won't boot**:
```bash
# Add log points in kernel/src/main.rs
log::info!("Checkpoint 1");
```

**Build fails**:
```bash
cargo clean && cargo build --release -v
rustup update nightly
```

---

## Quick Reference Card

Print and keep handy:

```
BUILD          cargo build --release
FORMAT         cargo fmt --all
LINT           cargo clippy -- -D warnings
CHECK          cargo check
RUN            ./run.ps1  (Windows) | qemu-system-riscv64 ... (Linux/macOS)
CLEAN          cargo clean
DISK           python3 create_ramdisk.py
DEBUG GDB      riscv64-unknown-elf-gdb + target remote localhost:1234

GOLDEN RULES (read CLAUDE.md)
1. Interface is sacred (libs/api/ needs 2x confirmation to change)
2. Owned buffers for async (not &mut [u8])
3. Multi-arch aware (use VAddr, PAddr, not u64)
4. Cells forbid unsafe (except kernel)
5. No mod.rs files (use foo.rs + foo/ dir)
6. Vi prefix for traits (ViFileSystem, ViDriver)
7. Trait objects for polymorphism (dyn Trait)
8. Implement Drop for resources

DOCS
  system-architecture.md  System design
  code-standards.md       How to write code
  CLAUDE.md               8 Coding Laws (start here)
  api-reference.md        Complete API reference
```

---

## Next Steps

### This Week

- [ ] Build and run ViOS successfully
- [ ] Read CLAUDE.md + system-architecture.md overview
- [ ] Trace one syscall through the code
- [ ] Join GitHub Discussions

### This Month

- [ ] Read all core docs (code-standards.md, system-architecture.md)
- [ ] Add a simple shell command (example above)
- [ ] Submit your first PR
- [ ] Understand Cellular SAS model

### Long-term

- [ ] Become a domain expert (kernel, HAL, cells, etc.)
- [ ] Mentor new contributors
- [ ] Design and implement a major feature

---

**Welcome to ViOS! Start small, ask questions, have fun. See you in the PRs.**
