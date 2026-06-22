# Cellos

[![CI](https://github.com/dxsl-org/vicell/actions/workflows/ci.yml/badge.svg)](https://github.com/dxsl-org/vicell/actions/workflows/ci.yml)
[![Ko-fi](https://img.shields.io/badge/Ko--fi-Donate-%23FF5E5B?logo=ko-fi)](https://ko-fi.com/dxsl_org)

A next-generation OS for the Edge-to-Cloud era. Software is organized as **Cells** (not processes) sharing one address space, isolated by the Rust type system rather than hardware MMU.

**Architecture**: Cellular Single Address Space (SAS) + Language-Based Isolation (LBI).  
**Status**: v1.x Mycelium · Active Stage: **G1 — Robot & Embedded**

---

## Quick Start

**Prerequisites**: Rust nightly · `qemu-system-riscv64` · PowerShell or WSL2

```powershell
git clone https://github.com/dxsl-org/vicell.git
cd vicell

# Build kernel (RV64, -pie, RUSTFLAGS handled by build.rs)
cargo build --release

# Generate FAT32 disk image and boot
./gen_disk.ps1
./run.ps1        # Ctrl+A X to exit QEMU
```

```
Cellos> echo hello
hello
Cellos> ls /bin
shell  vfs  net  input  compositor  hello  utils ...
Cellos> date
2026-06-08 00:00:00 UTC
Cellos> cat /proc/version
Cellos v1.x Mycelium riscv64
```

ARM64 target:

```powershell
./run-arm-virt.ps1
```

---

## What Works Today

| Subsystem | Status | Notes |
|-----------|--------|-------|
| **Kernel** | ✅ | Priority scheduler · RT TLSF heap · round-robin · spawn_pinned |
| **SMP** | ✅ | Phase 32 — hart boot (SBI HSM) · per-hart `ViHartLocal` via `tp` · cross-hart IPI · `WaitForEvent(217)` |
| **Memory** | ✅ | SAS · HHDM · frame allocator · cell quota · Storage 2.0 grant pages (64KB→16MB) |
| **ELF Loader** | ✅ | PIE + `R_RISCV_RELATIVE` · loaded from FAT32 `/bin/` |
| **VFS** | ✅ | RamFS + FAT32 · mkdir/rmdir/unlink/stat/readdir · 8-scenario e2e suite |
| **Shell** | ✅ | Pipes · redirects · tab completion · echo/cat/ls/pwd/cd/kill/ps |
| **Network** | ✅ | TCP/UDP/DNS/DHCP/MQTT (Phases A–E) · TLS 1.3 HTTPS demo |
| **Peripheral I/O** | ✅ | GPIO (PL061 · SiFive) · UART (PL011) · I2C bit-bang · SHT3x sensor demo |
| **IPC** | ✅ | Zero-copy owned buffers · typed IPC · syscall filter · large-buffer grant pages |
| **Reliability** | ✅ | Supervisor restart · guard pages · RT watchdog · `NotifyOnExit(204)` · zombie reaper |
| **RT Latency** | ✅ | All benchmarks pass on QEMU TCG · mtime-based · jitter in-cell |
| **RTC** | ✅ | Goldfish RTC (RV64/ARM64) · CMOS (x86_64) · `date` command |
| **WASM** | ✅ | Tier-2: wasmi + `vi.*` imports + fuel metering |
| **ViUI v2** | ✅ | Reactive Signal Tree · Dual-Layer DSL · GPU command buffer · embedded/robot readiness (P01–P10) |
| **Heap Snapshot** | ✅ | Instant-On: snapshot → restore |
| **RV32 Nano** | ✅ | QEMU S-mode boot verified · OpenSBI · SATP=0 |
| **AArch64** | ✅ boot | Boots to scheduler on QEMU virt |
| **x86_64** | ✅ boot | Boots to scheduler on QEMU q35 |
| Compositor | 📋 | Full GPU desktop — G2 |
| ARM64 full bring-up | 📋 | Beyond ring-3 smoke |
| WASM vi.* expand | 📋 | VFS+net+time+spawn imports |
| Hot migration | 📋 | Zero-downtime Cell live update — G2 |

---

## Architecture

```
Cellos/
├── kernel/             Nano-kernel: scheduler · loader · memory · IPC · syscalls
├── hal/
│   ├── traits/         ViArch · ViTimer · ViUart · ViGpio · ViI2c · ViMmc · ViDisplay
│   └── arch/           riscv/  arm/  x86/
├── libs/
│   ├── types/          VAddr · PAddr · ViError · ViResult
│   ├── api/            Kernel–Cell ABI (stable, Law 1 protected)
│   ├── ostd/           Cell std: alloc · mmio · font_atlas · sync
│   ├── viui/           ViUI v2: Signal tree · DSL · GPU renderer · widgets
│   └── viui-macros/    vi_design! proc macro
├── cells/
│   ├── apps/           init · shell · hello · utils · bench · viui-demo
│   │                   robot-demo · periph-demo · wasm · https-demo
│   ├── drivers/        disk · gpu · gpio · gpio-sifive · i2c-gpio · input · net · serial · wasm
│   └── services/       vfs · net · input · compositor · config · power
├── tools/
│   ├── vi-compiler/    .vi DSL → Rust codegen
│   └── viui-build/     build-time widget tree builder
└── docs/               Design specs (00–12) + developer guides
```

**Two product stages (overlay on technical phases):**

- **G1 — Robot & Embedded**: never-die · bounded RT · fault isolation · fast boot · peripheral I/O · small footprint. Target: ARM64/RV64 SBC + RV32 MCU sub-track.
- **G2 — Server & Specialized PC**: multi-core throughput · full desktop · untrusted code · hot migration · x86_64. Target: x86_64 + multi-core RV64/ARM64 servers.

---

## Build Targets

| Target | Status | Notes |
|--------|--------|-------|
| `riscv64gc-unknown-none-elf` | ✅ Primary | Full boot · all services |
| `aarch64-unknown-none` | ✅ Boots | Scheduler reached; full bring-up G1 next |
| `x86_64-unknown-none` | ✅ Boots | Scheduler reached; full bring-up G2 |
| `riscv32imc-unknown-none-elf` | ✅ Boot | Cellos-Nano · QEMU S-mode verified |

Build kernel with `RUSTFLAGS=-Crelocation-model=pic` (handled automatically). Cells stay non-PIC — do **not** put this in `.cargo/config.toml` globally.

---

## The 8 Coding Laws

1. **Interface is Sacred** — `libs/api/` changes require 2× user confirmation
2. **Owned Buffers for Async** — `async fn f(data: Box<[u8]>) -> Box<[u8]>`, never `&mut [u8]`
3. **Multi-Architecture** — use `VAddr`/`PAddr`; never hardcode pointer sizes
4. **Unsafe Management** — Cells: `#![forbid(unsafe_code)]`; Kernel: document every `unsafe` with `// SAFETY:`
5. **Modern Module Style** — `foo.rs` + `foo/` directory; `mod.rs` is forbidden
6. **Cellos Naming** — `Vi` prefix (Virtual Interface namespace) for public traits/types (`ViDriver`, `ViResult`); snake_case for files
7. **Trait Objects for Polymorphism** — `Arc<dyn ViDriver + Send + Sync>` at system boundaries
8. **RAII — Implement Drop** — all resources clean up explicitly; no process-based cleanup in SAS

Full rules: [CLAUDE.md](./CLAUDE.md) · [code-standards.md](./docs/code-standards.md)

---

## Documentation

| Document | Purpose |
|----------|---------|
| [system-architecture.md](./docs/system-architecture.md) | High-level design |
| [code-standards.md](./docs/code-standards.md) | Coding rules & 8 Laws |
| [patterns.md](./docs/patterns.md) | Common Rust patterns (global state, IPC, RAII) |
| [api-reference.md](./docs/api-reference.md) | Syscall table · trait reference |
| [project-roadmap.md](./docs/project-roadmap.md) | Phase progress & milestones |
| [getting-started.md](./docs/getting-started.md) | Setup guide |
| [security-model.md](./docs/security-model.md) | STRIDE model · known limitations |
| [hardware-dev-guide.md](./docs/hardware-dev-guide.md) | Real board workflow |
| [faq.md](./docs/faq.md) | Architecture Q&A |

**Design Specifications** (read before coding in a subsystem):

| Spec | Topic |
|------|-------|
| [00-context.md](./docs/specs/00-context.md) | Prime directive |
| [01-core.md](./docs/specs/01-core.md) | Cellular philosophy · linker |
| [02-memory.md](./docs/specs/02-memory.md) | SAS · HHDM · registry |
| [03-runtime.md](./docs/specs/03-runtime.md) | Async safety · owned buffers |
| [04-hardware.md](./docs/specs/04-hardware.md) | Multi-arch HAL |
| [05-application.md](./docs/specs/05-application.md) | Native · WASM · VM tiers |
| [06-graphics.md](./docs/specs/06-graphics.md) | ViUI · compositor · GPU |
| [07-networking.md](./docs/specs/07-networking.md) | Network stack |
| [09-vfs.md](./docs/specs/09-vfs.md) | VFS · filesystem |
| [11-shell.md](./docs/specs/11-shell.md) | Shell design |
| [12-reliability.md](./docs/specs/12-reliability.md) | Never-die · supervisor |

---

## Contributing

```bash
cargo check               # type check
cargo fmt --all           # format
cargo clippy -- -D warnings   # lint
cargo build --release     # build
cargo test --all          # run tests
```

**Before coding**: read [CLAUDE.md](./CLAUDE.md) and the relevant spec in `docs/specs/`.  
**Commit format**: `type(scope): description` — e.g. `feat(vfs): add readdir support`

---

## Acknowledgments

Cellos draws ideas from:
- **Theseus** (UC Santa Cruz) — live evolution, single address space
- **Asterinas** — FrameKernel safety abstractions
- **Tock** (Google) — embedded OS efficiency, hardware isolation traits
- **Redox OS** — Rust microkernel IPC patterns

---

**Version**: 1.x Mycelium · **Last Updated**: 2026-06-08 · **Maintained by**: lungmat8
