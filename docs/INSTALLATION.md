# ViOS Installation Guide

> Complete setup instructions for building and running ViOS on your development machine

## Table of Contents

1. [Prerequisites](#prerequisites)
2. [Quick Start](#quick-start)
3. [Detailed Installation](#detailed-installation)
4. [Building ViOS](#building-vios)
5. [Running in QEMU](#running-in-qemu)
6. [Debugging](#debugging)
7. [Troubleshooting](#troubleshooting)
8. [Platform-Specific Instructions](#platform-specific-instructions)

---

## Prerequisites

### Minimum Requirements

- **OS**: Linux, macOS, Windows (with WSL2 or MSYS2)
- **RAM**: 4GB minimum, 8GB recommended
- **Disk Space**: 2GB for tools + source code
- **CPU**: x86_64 or ARM64 (for cross-compilation to RISC-V)

### Required Tools

| Tool | Version | Purpose |
|------|---------|---------|
| **Rust** | nightly | Compiler and toolchain |
| **QEMU** | 7.0+ | RISC-V emulator |
| **Git** | 2.30+ | Source control |
| **Python** | 3.8+ | Build scripts (ramdisk creation) |

---

## Quick Start

**For impatient developers:**

```bash
# 1. Install Rust nightly
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup default nightly

# 2. Clone repository
git clone https://github.com/your-org/vios.git
cd vios

# 3. Install Rust components
rustup component add rust-src rustfmt clippy
rustup target add riscv64gc-unknown-none-elf

# 4. Install QEMU (Linux/macOS)
# Ubuntu/Debian:
sudo apt install qemu-system-riscv64

# macOS:
brew install qemu

# Windows (MSYS2):
pacman -S mingw-w64-x86_64-qemu

# 5. Build kernel
cargo build --release

# 6. Create disk image
python create_ramdisk.py

# 7. Run in QEMU
./run.ps1  # Windows PowerShell
# OR
qemu-system-riscv64 -machine virt -cpu rv64 -m 128M -nographic \
    -bios default -kernel target/riscv64gc-unknown-none-elf/release/vios-kernel \
    -drive file=disk.img,format=raw,if=none,id=hd0 \
    -device virtio-blk-device,drive=hd0
```

**Expected Output**:
```
VIOS K MAIN ENTRY
[INFO] Kernel started (Hart: 0, DTB: 0x87000000)
[INFO] Frame allocator initialized
[INFO] Paging initialized at 0x80400000
[INFO] Heap initialized
...
ViOS Shell v0.2.0
vios>
```

---

## Detailed Installation

### Step 1: Install Rust Toolchain

#### Linux / macOS

```bash
# Install rustup (Rust installer)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Follow prompts (default installation is fine)

# Reload shell or run:
source "$HOME/.cargo/env"

# Install nightly toolchain
rustup toolchain install nightly
rustup default nightly

# Verify installation
rustc --version
# Should show: rustc 1.xx.0-nightly (...)
```

#### Windows

**Option A: WSL2** (Recommended)
```powershell
# Install WSL2
wsl --install

# Inside WSL2, follow Linux instructions above
```

**Option B: MSYS2**
```powershell
# Download and install MSYS2 from https://www.msys2.org/

# In MSYS2 terminal:
pacman -S mingw-w64-x86_64-rust

# Then follow Linux instructions for rustup
```

#### Add Required Components

```bash
# Rust source code (for no_std development)
rustup component add rust-src

# Code formatter
rustup component add rustfmt

# Linter
rustup component add clippy

# Add RISC-V target
rustup target add riscv64gc-unknown-none-elf

# Verify components
rustup component list --installed
```

---

### Step 2: Install QEMU

QEMU is required to run ViOS without physical RISC-V hardware.

#### Ubuntu / Debian

```bash
sudo apt update
sudo apt install qemu-system-riscv64 qemu-system-misc

# Verify installation
qemu-system-riscv64 --version
# Should show: QEMU emulator version 7.x or higher
```

#### Fedora / Red Hat

```bash
sudo dnf install qemu-system-riscv

# Verify
qemu-system-riscv64 --version
```

#### macOS

```bash
# Install Homebrew if not already installed
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"

# Install QEMU
brew install qemu

# Verify
qemu-system-riscv64 --version
```

#### Windows

**Option A: MSYS2** (Recommended)
```bash
# In MSYS2 MINGW64 terminal:
pacman -S mingw-w64-x86_64-qemu

# Verify
qemu-system-riscv64 --version
```

**Option B: Native Windows**
```powershell
# Download QEMU for Windows from https://qemu.weilnetz.de/w64/

# Install to C:\Program Files\qemu

# Add to PATH or use full path:
& "C:\Program Files\qemu\qemu-system-riscv64.exe" --version
```

---

### Step 3: Install Additional Tools

#### Python (for build scripts)

```bash
# Ubuntu/Debian
sudo apt install python3 python3-pip

# macOS
brew install python3

# Windows (MSYS2)
pacman -S mingw-w64-x86_64-python

# Verify
python3 --version
```

#### GDB (for debugging)

```bash
# Ubuntu/Debian
sudo apt install gdb-multiarch

# macOS
brew install riscv64-elf-gdb

# Arch Linux
sudo pacman -S riscv64-linux-gnu-gdb

# Verify
riscv64-unknown-elf-gdb --version  # or gdb-multiarch
```

---

### Step 4: Clone ViOS Repository

```bash
# Clone via HTTPS
git clone https://github.com/your-org/vios.git
cd vios

# OR clone via SSH (if you have keys set up)
git clone git@github.com:your-org/vios.git
cd vios

# Check current branch
git branch
# Should be on 'main'

# Check status
git status
```

---

## Building ViOS

### Build Profiles

**Debug Build** (faster compilation, larger binary, debug symbols):
```bash
cargo build
# Output: target/riscv64gc-unknown-none-elf/debug/vios-kernel
```

**Release Build** (slower compilation, smaller binary, optimized):
```bash
cargo build --release
# Output: target/riscv64gc-unknown-none-elf/release/vios-kernel
```

### Build Options

```bash
# Build specific package
cargo build -p vios-kernel

# Build all workspace members
cargo build --workspace

# Build with verbose output
cargo build -v

# Clean build artifacts
cargo clean

# Check code without building (fast)
cargo check

# Run linter
cargo clippy -- -D warnings

# Format code
cargo fmt --all

# View build timings
cargo build --timings
```

### Build Output

After successful build:
```
target/
└── riscv64gc-unknown-none-elf/
    ├── debug/
    │   ├── vios-kernel        # Debug binary
    │   └── vios-kernel.d      # Dependency info
    └── release/
        ├── vios-kernel        # Release binary (use this)
        └── vios-kernel.d
```

---

## Running in QEMU

### Creating Disk Image

ViOS requires a FAT32 disk image for the filesystem:

```bash
# Using Python script (recommended)
python3 create_ramdisk.py

# This creates disk.img (40MB FAT32 disk)
```

**Manual Creation** (Linux/macOS):
```bash
# Create 40MB blank file
dd if=/dev/zero of=disk.img bs=1M count=40

# Format as FAT32
mkfs.vfat disk.img

# Mount and add files (optional)
mkdir -p /tmp/vios-mount
sudo mount -o loop disk.img /tmp/vios-mount
sudo cp -r files/* /tmp/vios-mount/
sudo umount /tmp/vios-mount
```

### Running Kernel

#### Using PowerShell Script (Windows)

```powershell
.\run.ps1
```

#### Using Manual Command (Linux/macOS)

```bash
qemu-system-riscv64 \
    -machine virt \
    -cpu rv64,c=true \
    -smp 1 \
    -m 128M \
    -nographic \
    -serial mon:stdio \
    -bios default \
    -kernel target/riscv64gc-unknown-none-elf/release/vios-kernel \
    -drive file=disk.img,format=raw,if=none,id=hd0 \
    -device virtio-blk-device,drive=hd0
```

#### QEMU Parameters Explained

| Parameter | Value | Purpose |
|-----------|-------|---------|
| `-machine` | virt | Generic RISC-V virtual machine |
| `-cpu` | rv64,c=true | RISC-V 64-bit with compressed instructions |
| `-smp` | 1 | Number of CPU cores (single core for now) |
| `-m` | 128M | RAM size (128 megabytes) |
| `-nographic` | - | No graphical window, use terminal only |
| `-serial` | mon:stdio | Serial console to stdout |
| `-bios` | default | Use OpenSBI firmware (bundled with QEMU) |
| `-kernel` | ... | Path to kernel ELF file |
| `-drive` | ... | Disk image configuration |
| `-device` | virtio-blk-device | VirtIO block device |

### Exiting QEMU

```bash
# Press Ctrl+A, then X
# OR
# If that doesn't work, press Ctrl+C multiple times
```

---

## Debugging

### Starting QEMU in Debug Mode

```bash
# Start QEMU with GDB server (waits for debugger)
qemu-system-riscv64 \
    -machine virt -cpu rv64 -m 128M \
    -nographic -bios default \
    -kernel target/riscv64gc-unknown-none-elf/debug/vios-kernel \
    -drive file=disk.img,format=raw,if=none,id=hd0 \
    -device virtio-blk-device,drive=hd0 \
    -s -S

# -s: Start GDB server on localhost:1234
# -S: Pause execution until debugger connects
```

### Connecting GDB

**In another terminal:**

```bash
# Start GDB with kernel binary
riscv64-unknown-elf-gdb target/riscv64gc-unknown-none-elf/debug/vios-kernel

# Or use gdb-multiarch
gdb-multiarch target/riscv64gc-unknown-none-elf/debug/vios-kernel
```

**GDB Commands:**

```gdb
# Connect to QEMU
(gdb) target remote localhost:1234

# Set breakpoint at kernel entry
(gdb) break kmain

# Continue execution
(gdb) continue

# When breakpoint hits:
(gdb) info registers    # Show CPU registers
(gdb) backtrace         # Show call stack
(gdb) step              # Step one instruction
(gdb) next              # Step over function calls
(gdb) print variable    # Print variable value

# Examine memory
(gdb) x/10x 0x80200000  # Show 10 words at address

# Quit
(gdb) quit
```

### VS Code Debugging

Create `.vscode/launch.json`:

```json
{
    "version": "0.2.0",
    "configurations": [
        {
            "type": "gdb",
            "request": "launch",
            "name": "Debug ViOS Kernel",
            "target": "${workspaceFolder}/target/riscv64gc-unknown-none-elf/debug/vios-kernel",
            "cwd": "${workspaceFolder}",
            "gdbpath": "riscv64-unknown-elf-gdb",
            "autorun": [
                "target remote localhost:1234"
            ],
            "preLaunchTask": "start-qemu-debug"
        }
    ]
}
```

---

## Troubleshooting

### Build Errors

#### Error: "can't find crate for `core`"

**Solution**:
```bash
rustup component add rust-src
```

#### Error: "linking with `rust-lld` failed"

**Solution**:
```bash
# Make sure target is installed
rustup target add riscv64gc-unknown-none-elf

# Try cleaning and rebuilding
cargo clean
cargo build
```

#### Error: "package ... requires `edition = "2021"`"

**Solution**:
```bash
# Update Rust to latest nightly
rustup update nightly
```

### Runtime Errors

#### QEMU doesn't start / "qemu-system-riscv64: command not found"

**Solution**:
```bash
# Linux: Install QEMU
sudo apt install qemu-system-riscv64

# macOS: Install QEMU
brew install qemu

# Windows: Add QEMU to PATH or use full path
```

#### Kernel boots but hangs immediately

**Check**:
- Is `disk.img` present? Run `python create_ramdisk.py`
- Is OpenSBI loading? You should see OpenSBI banner
- Try adding `-d int` to QEMU command for interrupt logs

#### "triple `riscv64gc-unknown-none-elf` is not supported"

**Solution**:
```bash
# Update Rust and add target again
rustup update nightly
rustup target add riscv64gc-unknown-none-elf
```

### Performance Issues

#### Build is very slow

**Solutions**:
- Use `cargo check` instead of `cargo build` for fast iteration
- Use incremental compilation (should be default)
- Add to `.cargo/config.toml`:
  ```toml
  [build]
  incremental = true
  ```
- Use `cargo build --release` only when needed

#### QEMU is slow

**Solutions**:
- Reduce RAM: `-m 64M` instead of `-m 128M`
- Use KVM acceleration (Linux only, x86 host):
  ```bash
  # Not applicable for RISC-V emulation on x86 host
  ```

---

## Platform-Specific Instructions

### Linux (Ubuntu/Debian)

```bash
# Full installation script
sudo apt update
sudo apt install -y curl build-essential qemu-system-riscv64 gdb-multiarch python3

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# Setup Rust
rustup default nightly
rustup component add rust-src rustfmt clippy
rustup target add riscv64gc-unknown-none-elf

# Clone and build
git clone https://github.com/your-org/vios.git
cd vios
python3 create_ramdisk.py
cargo build --release

# Run
qemu-system-riscv64 -machine virt -cpu rv64 -m 128M -nographic \
    -bios default -kernel target/riscv64gc-unknown-none-elf/release/vios-kernel \
    -drive file=disk.img,format=raw,if=none,id=hd0 -device virtio-blk-device,drive=hd0
```

### macOS

```bash
# Install Homebrew
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"

# Install tools
brew install rustup qemu python3

# Setup Rust
rustup-init
rustup default nightly
rustup component add rust-src rustfmt clippy
rustup target add riscv64gc-unknown-none-elf

# Clone and build
git clone https://github.com/your-org/vios.git
cd vios
python3 create_ramdisk.py
cargo build --release

# Run
qemu-system-riscv64 -machine virt -cpu rv64 -m 128M -nographic \
    -bios default -kernel target/riscv64gc-unknown-none-elf/release/vios-kernel \
    -drive file=disk.img,format=raw,if=none,id=hd0 -device virtio-blk-device,drive=hd0
```

### Windows (MSYS2)

```bash
# Install MSYS2 from https://www.msys2.org/

# In MSYS2 MINGW64 terminal:
pacman -Syu
pacman -S mingw-w64-x86_64-rust mingw-w64-x86_64-qemu git python3

# Setup Rust
rustup default nightly
rustup component add rust-src rustfmt clippy
rustup target add riscv64gc-unknown-none-elf

# Clone and build
git clone https://github.com/your-org/vios.git
cd vios
python create_ramdisk.py
cargo build --release

# Run using PowerShell script
./run.ps1
```

---

## Continuous Integration Setup

### GitHub Actions Example

Create `.github/workflows/build.yml`:

```yaml
name: Build and Test

on: [push, pull_request]

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3

      - name: Install Rust
        uses: actions-rs/toolchain@v1
        with:
          toolchain: nightly
          components: rust-src, rustfmt, clippy
          override: true

      - name: Add RISC-V target
        run: rustup target add riscv64gc-unknown-none-elf

      - name: Check formatting
        run: cargo fmt --all -- --check

      - name: Run clippy
        run: cargo clippy --all -- -D warnings

      - name: Build kernel
        run: cargo build --release --verbose

      - name: Upload artifact
        uses: actions/upload-artifact@v3
        with:
          name: vios-kernel
          path: target/riscv64gc-unknown-none-elf/release/vios-kernel
```

---

## Next Steps

After successful installation:

1. **Read the documentation**:
   - [Architecture](./ARCHITECTURE.md) - Understand system design
   - [Coding Guide](./CODING_GUIDE.md) - Learn coding conventions
   - [Onboarding Guide](./ONBOARDING.md) - Get started with development

2. **Run examples**:
   ```bash
   # Try the shell
   # At ViOS prompt, type: help
   ```

3. **Make changes**:
   - Edit `kernel/src/main.rs`
   - Rebuild: `cargo build`
   - Test in QEMU

4. **Join the community**:
   - GitHub Discussions
   - Discord/Matrix chat
   - Contribute!

---

**Last Updated**: 2026-01-07
**Version**: 0.2.0
**Maintainer**: ViOS Team
