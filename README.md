# ViOS (Jarvis Hybrid OS)

[![CI](https://github.com/vi-group/ViCell/actions/workflows/ci.yml/badge.svg)](https://github.com/vi-group/ViCell/actions/workflows/ci.yml)

A next-generation operating system designed for the Edge-to-Cloud era, combining innovations from **Theseus** (Live Evolution), **Asterinas** (FrameKernel Safety), and **Tock** (Embedded Efficiency).

**Key Innovation**: Cellular Single Address Space (SAS) using Language-Based Isolation (LBI) via Rust's type system. Software is organized as **Cells** (not processes) sharing one address space, isolated by the Rust compiler rather than hardware MMU.

**Status**: v0.2.0 (Mycelium Era) — Phase 1 Core Stability in progress

---

## Quick Start

### Prerequisites
- Rust nightly (2024+): `rustup install nightly`
- QEMU with RISC-V support: `qemu-system-riscv64`
- Linux/macOS/WSL2 (Windows support via bash)

### Build & Run

```bash
# Clone repository
git clone https://github.com/vi-group/vios.git
cd vios

# Build kernel for RV64
cargo build --release

# Generate disk image
./gen_disk.ps1  # or run.sh on Linux

# Run in QEMU (non-graphical)
./run.ps1       # Ctrl+A X to exit
# or: qemu-system-riscv64 -machine virt -cpu rv64 -smp 1 -m 128M \
#     -nographic -kernel target/riscv64gc-unknown-none-elf/release/vios-kernel \
#     -drive file=disk_v3.img,format=raw,id=hd0 \
#     -device virtio-blk-device,drive=hd0
```

### First Commands
```
viosh> echo "Hello ViOS"
Hello ViOS

viosh> ls /bin
hello
cat
ls
pwd

viosh> cat /etc/version
v0.2.0 Mycelium Era
```

---

## Documentation

**New to ViOS?** Start here:

| Document | Purpose |
|----------|---------|
| [system-architecture.md](./docs/system-architecture.md) | High-level design for developers |
| [codebase-summary.md](./docs/codebase-summary.md) | File structure & metrics |
| [code-standards.md](./docs/code-standards.md) | Coding rules & conventions (8 Laws) |
| [project-overview-pdr.md](./docs/project-overview-pdr.md) | PDR + detailed requirements |
| [project-roadmap.md](./docs/project-roadmap.md) | Phase progress & milestones |

**Deep Dives:**

| Document | Level | Topic |
|----------|-------|-------|
| [getting-started.md](./docs/getting-started.md) | Beginner | Setup, build, first contribution |
| [api-reference.md](./docs/api-reference.md) | Reference | Full trait & syscall reference |
| [patterns.md](./docs/patterns.md) | Intermediate | Common code patterns |
| [security-model.md](./docs/security-model.md) | Advanced | STRIDE model & known limitations |
| [faq.md](./docs/faq.md) | All levels | Architecture questions answered |

**Design Specifications** (read before coding):

| Doc | Topic |
|-----|-------|
| [specs/00-context.md](./docs/specs/00-context.md) | Prime directive & overview |
| [specs/01-core.md](./docs/specs/01-core.md) | Cellular philosophy & linker |
| [specs/02-memory.md](./docs/specs/02-memory.md) | SAS, HHDM, memory registry |
| [specs/03-runtime.md](./docs/specs/03-runtime.md) | Async safety & owned buffers |
| [specs/04-hardware.md](./docs/specs/04-hardware.md) | Multi-arch HAL (RV, ARM, x86) |
| [specs/05-application.md](./docs/specs/05-application.md) | Native/WASM/VM applications |
| [specs/06-graphics.md](./docs/specs/06-graphics.md) | Graphics & compositor |
| [specs/07-networking.md](./docs/specs/07-networking.md) | Network stack |
| [specs/08-power.md](./docs/specs/08-power.md) | Power management |
| [specs/09-vfs.md](./docs/specs/09-vfs.md) | Filesystem (VFS) |
| [specs/10-testing.md](./docs/specs/10-testing.md) | Testing strategy |
| [specs/11-shell.md](./docs/specs/11-shell.md) | Shell design |

---

## Project Structure

```
vios/
├── kernel/             Nano kernel (~8,700 LOC)
├── hal/                Hardware Abstraction Layer
│   ├── core/           Facade
│   ├── traits/         Pure trait definitions
│   └── arch/           riscv (IMPL), arm/x86 (STUB)
├── libs/               Public API (Stable ABI)
│   ├── types/          VAddr, PAddr, ViError, etc.
│   ├── api/            Kernel-Cell boundary traits
│   └── ostd/           Cells' standard library
├── cells/              User-space software
│   ├── apps/           Applications (init, shell, hello, utils)
│   ├── drivers/        Hardware drivers (disk, gpu, input, net, serial, wasm)
│   ├── services/       System services (vfs, config, compositor, etc.)
│   └── runtimes/       VMs (Lua 5.4, MicroPython 1.24.1)
├── docs/               Design specs & developer guides
└── tests/              Architecture validation suite
```

---

## Key Features

✅ **RISC-V 64-bit (RV64)** HAL with SV39 paging, PLIC, SBI  
✅ **Nano-kernel** (~12,600 LOC) with round-robin scheduler  
✅ **Capability-based IPC** (zero-copy owned-buffer passing)  
✅ **ELF loader** — external cells loaded from `/bin/` via PIE + R_RISCV_RELATIVE  
✅ **VFS** — RamFS + FAT32 IPC (mkdir/rmdir/unlink/stat/readdir)  
✅ **Interactive shell** with echo, cat, ls, pwd, cd  
✅ **Lua 5.4 + MicroPython 1.24.1** runtime bindings  
✅ **VirtIO block** — hang fixed; keyboard deadlock fixed  
✅ **AArch64 + x86_64 HALs** — boot to Ring-3 smoke in QEMU  
✅ **RV32 + AArch32** HAL trait implementations  
✅ **Security infrastructure** — STRIDE model, fuzzing, capability audit  
🚧 **FAT32 on VirtIO** — FatFsAdapter (in progress)  
🚧 **Input / network / compositor services** (in progress)

---

## The 8 Coding Laws

All developers must follow these non-negotiable rules:

1. **Interface is Sacred** — Changes to `libs/api/` require 2x confirmation
2. **Owned Buffers for Async** — Never `&mut [u8]` across async boundaries
3. **Multi-Architecture Awareness** — Use `VAddr`/`PAddr`, never hardcode pointer sizes
4. **Unsafe Management** — Cells: forbid unsafe; Kernel: document all unsafe with `// SAFETY:`
5. **Modern Module Structure** — Use `foo.rs` + `foo/` folder, never `mod.rs`
6. **ViOS Naming Convention** — `Vi` prefix for traits/types, snake_case for modules
7. **Trait Objects for Polymorphism** — Dynamic dispatch at system boundaries
8. **RAII - Implement Drop** — All resources must clean up explicitly

See [CLAUDE.md](./CLAUDE.md) for quick reference.

---

## Build Targets

| Target | Status | HAL | Notes |
|--------|--------|-----|-------|
| `riscv64gc-unknown-none-elf` | ✅ WORKING | RV64 SV39 | Primary target |
| `aarch64-unknown-none` | 🚧 STUB | Arm64 | Implementation in progress |
| `x86_64-unknown-none` | 🚧 STUB | x86_64 | Implementation planned |

---

## Architecture Highlights

- **Single Address Space**: No process boundaries, reduced context-switch overhead
- **Language-Based Isolation**: Rust compiler enforces Cell isolation
- **Capability Objects**: Fine-grained access control via syscalls
- **Async/Await**: Native async runtime for I/O-bound operations
- **Multi-Architecture**: Feature flags for RV32/RV64, ARM32/ARM64, x86_64

---

## Current Phase (v0.2.0 — Mycelium Era)

**Phase 1: Core Stability** (April—June 2026)

Priority fixes:
1. ✅ Architecture validation (10/10 score)
2. 🚧 VirtIO block device hang (blocking disk access)
3. 🚧 Keyboard input deadlock (single-key limitation)
4. 📋 Multi-architecture HAL (RV64 done, ARM/x86 next)
5. 📋 External ELF loading (load `/bin/*` binaries from disk)

See [project-roadmap.md](./docs/project-roadmap.md) for detailed milestones.

---

## Contributing

ViOS is open-source (see [LICENSE](./LICENSE) when available).

**Before coding:**
1. Read [CLAUDE.md](./CLAUDE.md) — auto-loaded guidelines
2. Read relevant spec in `docs/specs/0X-*.md`
3. Read [code-standards.md](./docs/code-standards.md) — coding rules
4. Create a branch: `git checkout -b feature/my-feature`

**Development workflow:**
```bash
# Check code
cargo check

# Format
cargo fmt --all

# Lint
cargo clippy -- -D warnings

# Build
cargo build --release

# Run tests
cargo test --all --release
```

**Submit PR:**
- Title: `feat(scope): description` or `fix(scope): description`
- Reference related issues: `Fixes #123`
- Ensure CI passes (lint, build, tests)

---

## Community

| Resource | Purpose |
|----------|---------|
| [GitHub Discussions](../../discussions) | Questions, ideas, show-and-tell |
| [GitHub Issues](../../issues) | Bug reports and feature requests |
| [`good-first-issue`](../../issues?q=label%3Agood-first-issue) | Curated starting points for new contributors |

**New contributor?**

1. Run `./scripts/dev-setup.sh` (Linux/macOS) or `.\scripts\dev-setup.ps1` (Windows)
2. Read [docs/getting-started.md](docs/getting-started.md) — setup guide + common errors
3. Read [CONTRIBUTING.md](CONTRIBUTING.md) — step-by-step first PR walkthrough
4. Browse [docs/faq.md](docs/faq.md) — architecture questions answered
5. Check [docs/project-roadmap.md](docs/project-roadmap.md) — where the project is headed
6. Review [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) — community standards

We aim to respond to PRs and new issues within **3 business days**.

---

## License

ViOS is designed to be open-source under a permissive license (MIT or Apache 2.0, details TBD).

---

## Acknowledgments

ViOS builds on insights from:
- **Theseus** (UC Santa Cruz) — Live evolution, capability-based security
- **Asterinas** — FrameKernel safety abstractions
- **Tock** (Google) — Embedded OS efficiency, hardware abstraction

---

**Version**: 0.2.1 (Mycelium Era)  
**Last Updated**: 2026-05-29  
**Maintained by**: ViOS Team
