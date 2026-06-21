# ViCell App Development Guide

> How to choose a development model and write applications for ViCell.
> For syscall reference, see [api-reference.md](api-reference.md);
> for kernel internals, see [system-architecture.md](system-architecture.md).

**Version**: v0.3.0 | **Last updated**: 2026-06-19

---

## What is a Cell App?

A ViCell application runs as a **Cell** — a language-isolated unit in a single address space (SAS). There is no process fork; instead, you declare which language and isolation tier you want. The kernel and type system enforce boundaries, not the MMU.

---

## Development Tiers

| Tier | Language(s) | Entry Point | Isolation | Performance | When to Use |
|------|-------------|-------------|-----------|-------------|------------|
| **Tier 1** | Rust | `ostd::app_entry!` | Language-based (Cells `#![forbid(unsafe)]`) | Native | Default; maximum safety + speed. |
| **Tier 1 + SDK L1** | Rust | `AppContext::run()` | Language-based | Native | Need VFS, network, or IPC services. |
| **Tier 1 + ViUI** | Rust | Signal API + `.vi` DSL | Language-based | Native | Building UIs, dashboards, embedded GUIs. |
| **Tier 1 Extended** | Rust | `SiloHandle::connect()` | Hardware isolation (Silo) | ~10% overhead | Cryptographic keys, secrets. **G2+ only**. |
| **Tier 1b** | C / C++ / Zig | extern "C" via mlibc | Language-based | Native | Tier A (POSIX shim) or Tier B (full mlibc). |
| **Tier 1b** | Lua | `require()`/REPL | Lua VM interpreter | ~50× slower | Scripts, CLI tools, quick prototyping. |
| **Tier 3b** | Linux | Standard `/bin/app` | VMM + full kernel | ~10–15% overhead | Legacy fork-heavy code; untrusted workloads. **G2 only**. |

---

## Decision Tree: Which Tier?

```
┌─ "I have existing C/C++/Zig code"
│  └─ Use Tier 1b (POSIX shim or mlibc)
│
├─ "I want to write Rust"
│  ├─ "Need VFS, network, or IPC?"
│  │  └─ Use Tier 1 + SDK L1
│  ├─ "Building a UI or dashboard?"
│  │  └─ Use Tier 1 + ViUI
│  ├─ "Handling cryptographic keys?"
│  │  └─ Use Tier 1 Extended (Silo, G2+)
│  └─ "Just syscalls and linked libraries?"
│     └─ Use Tier 1 (bare)
│
├─ "I want quick scripting / dynamic code"
│  └─ Use Tier 1b Lua
│
└─ "I have a legacy Linux binary / fork() is essential"
   └─ Use Tier 3b (Linux VM, G2+)
```

---

## Guides by Tier

- **[Tier 1 Rust (Bare)](guides/tier1-rust-bare.md)** — Minimal entry point, syscall allowlists, manifest declaration.
- **[Tier 1 Rust + SDK L1](guides/tier1-rust-sdk.md)** — AppContext, VFS/network clients, service discovery.
- **[Tier 1 Rust + ViUI](guides/viui-guide.md)** — Signal API, .vi DSL, compositor surfaces (see `system-architecture.md` §6).
- **[Tier 1 Extended (Silo)](guides/tier1-silo.md)** — SiloHandle, cryptographic isolation, ARM64/x86 only.
- **[Tier 1b C/Zig (POSIX + mlibc)](guides/tier1b-c-zig.md)** — Two-tier C ABI: Tier A (POSIX shim) vs Tier B (full mlibc).
- **[Tier 1b Lua](guides/tier1b-lua.md)** — Interpreter cell, VFS bindings, restricted stdlib.
- **[Tier 3b Linux VM](guides/tier3b-linux-vm.md)** — Full kernel in hypervisor, G2 only.

---

## SAS Laws Apply to All Cells

All Cells (regardless of tier) must respect the **8 Coding Laws** in [code-standards.md](code-standards.md):

| Law | Rule |
|-----|------|
| **Law 2** | Owned buffers (`Box<[u8]>`) across async; never `&mut [u8]`. |
| **Law 4** | Cells forbid `unsafe` (no exceptions for `#[no_mangle] main` in app_entry!). |
| **Law 5** | No `mod.rs` files — use `foo.rs` parallel to `foo/`. |
| **Law 8** | Implement `Drop` for all resources; no process cleanup. |

---

## Build & Run

```bash
# In a Cell directory (e.g., cells/apps/hello-cell):
cargo build --release --target riscv64gc-unknown-none-elf

# Run on QEMU:
./run.ps1   # Uses scripts/run-qemu-riscv64.sh internally
```

For multi-arch builds (ARM64, x86), see [getting-started.md](getting-started.md) § Build.

---

## Examples

- **Tier 1 bare**: `cells/apps/hello-cell/src/main.rs`
- **Tier 1 + SDK L1**: `cells/apps/sdk-demo/src/main.rs`
- **Tier 1 + ViUI**: `cells/apps/robot-dashboard/src/main.rs`
- **Tier 1 Extended**: `cells/apps/silo-test/src/main.rs`
- **Tier 1b C**: `cells/apps/mlibc-smoke/src/main.rs`
- **Tier 1b POSIX**: `cells/apps/posix-shim-test/src/main.rs`
- **Tier 1b Lua**: `cells/runtimes/lua/src/main.rs`

---

## Next Steps

1. Pick your tier from the **Decision Tree** above.
2. Read the corresponding **Guide**.
3. Copy a canonical example from the list above.
4. Adapt for your use case.
5. See [api-reference.md](api-reference.md) for syscall details.

---

## FAQ

**Q: Can I use the Rust standard library?**
A: No. Use `ostd` instead (all you need is bundled). For C, use mlibc (Tier 1b).

**Q: Do I need to write unsafe code?**
A: No. Cells forbid `unsafe` at the crate root (Law 4). Only syscall entry points (`app_entry!` generated code) use it, and it is isolated.

**Q: How do I talk to other Cells?**
A: Use IPC. See [api-reference.md](api-reference.md) § IPC for syscalls (`sys_send`, `sys_recv`); Tier 1 + SDK L1 provides ergonomic wrappers.

**Q: Can I spawn other Cells?**
A: Only `/bin/*` Cells with `spawn = true` in the manifest. See Phase 30 (project-roadmap.md).

**Q: What about real-time performance?**
A: Tier 1 is native (~1 μs syscall latency on QEMU). Use `sys_heartbeat()` for watchdog-style deadlines.

