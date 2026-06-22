# Cellos Documentation Index

**Version**: v0.2.1-dev (Mycelium Era) | **Last updated**: 2026-06-03

---

## Start Here

| File | Purpose |
|------|---------|
| [getting-started.md](getting-started.md) | Setup, build, run, first contribution |
| [app-development-guide.md](app-development-guide.md) | Write/build/run/test a Cell application (worked examples) |
| [codebase-summary.md](codebase-summary.md) | Quick reference: LOC, crates, features |
| [faq.md](faq.md) | Common questions about architecture |

---

## Project

| File | Purpose |
|------|---------|
| [project-overview-pdr.md](project-overview-pdr.md) | Vision, requirements, success metrics |
| [project-roadmap.md](project-roadmap.md) | Milestones, phase status, blockers |
| [project-changelog.md](project-changelog.md) | History of changes per phase |

---

## Architecture & Standards

| File | Purpose |
|------|---------|
| [system-architecture.md](system-architecture.md) | System layers, kernel, HAL, IPC |
| [code-standards.md](code-standards.md) | The 8 Coding Laws + conventions |
| [patterns.md](patterns.md) | Common Rust patterns for Cellos |
| [security-model.md](security-model.md) | STRIDE analysis, known limitations |

---

## Reference

| File | Purpose |
|------|---------|
| [api-reference.md](api-reference.md) | Syscall ABI, trait definitions, examples |
| [performance-report.md](performance-report.md) | Benchmarking targets and methodology |

---

## Feature Guides

| File | Purpose |
|------|---------|
| [scripting-guide.md](scripting-guide.md) | Lua 5.4 + MicroPython usage |
| [hotswap-guide.md](hotswap-guide.md) | Live Cell upgrade protocol |
| [vfs-api.md](vfs-api.md) | VFS IPC opcodes and protocol |
| [network-api.md](network-api.md) | Network service IPC, DHCP, socket API |
| [display-api.md](display-api.md) | Compositor IPC, surface lifecycle |
| [input-api.md](input-api.md) | Input service IPC, KeySym, focus |

---

## Design Specifications

Internal design docs — read before implementing a subsystem.

| File | Topic |
|------|-------|
| [specs/00-context.md](specs/00-context.md) | Prime directive, coding laws, workflow |
| [specs/00-fork.md](specs/00-fork.md) | Strategy for forking external code |
| [specs/01-core.md](specs/01-core.md) | Cellular model, symbol table, security |
| [specs/02-memory.md](specs/02-memory.md) | SAS layout, quota, metadata registry |
| [specs/03-runtime.md](specs/03-runtime.md) | IPC, async/await, hot-swap, boot optimization |
| [specs/04-hardware.md](specs/04-hardware.md) | Multi-arch HAL, WASM drivers, SMP |
| [specs/05-application.md](specs/05-application.md) | 3-tier isolation: Native / WASM / Hypervisor |
| [specs/06-graphics.md](specs/06-graphics.md) | Compositor, framebuffer, input dispatch |
| [specs/07-networking.md](specs/07-networking.md) | Network stack, smoltcp, zero-copy |
| [specs/08-power.md](specs/08-power.md) | Power states, hibernation, thermal |
| [specs/09-vfs.md](specs/09-vfs.md) | VFS traits, dual-filesystem, direct I/O |
| [specs/10-testing.md](specs/10-testing.md) | Test strategy, QEMU harness, coverage |
| [specs/11-shell.md](specs/11-shell.md) | Shell design, ELF execution, zero-copy ls |
