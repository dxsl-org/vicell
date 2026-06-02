# ViOS — Frequently Asked Questions

---

## 1. What is ViOS?

ViOS is a **Cellular Single Address Space (SAS) operating system** written in Rust.
All code — kernel, drivers, and applications — shares one virtual address space.
Isolation is enforced by **Rust's type system and ownership model** (Language-Based
Isolation, LBI), not by hardware page tables.  Software is organized as **Cells**
rather than traditional processes.

---

## 2. Why "Cellular"?

A Cell is the fundamental unit of ViOS software, analogous to a process in Linux but
much lighter.  Cells share the same address space and communicate via zero-copy IPC
(ownership transfer), making the design similar to a mycelium network — many
independent organisms sharing the same substrate.

---

## 3. How is ViOS different from Redox, seL4, or Theseus?

| | ViOS | Redox | seL4 | Theseus |
|-|------|-------|------|---------|
| Isolation | Rust type system (LBI) | Hardware MMU | Hardware MMU + formal proof | Language + type system |
| IPC | Zero-copy owned-buffer | Message passing | Capability IPC | No-copy via type safety |
| Language | Rust (no_std) | Rust | C (kernel), Rust user | Rust |
| Focus | Edge-to-Cloud, Cellular SAS | POSIX-compatible | High-assurance embedded | Live evolution / hot-swap |

ViOS is most similar to Theseus in spirit but targets a broader hardware range and
prioritizes zero-copy IPC performance over formal verification.

---

## 4. Why Rust nightly?

ViOS uses several nightly-only features required for bare-metal `no_std` programming:

- `-Z build-std=core,alloc` — build the standard library for bare-metal targets
- `#![feature(custom_test_frameworks)]` — test runner in `no_std` kernels
- `#![feature(naked_functions)]` — assembly stubs without prologue/epilogue

The toolchain version is pinned in `rust-toolchain.toml`; updates are deliberate and
tested.

---

## 5. Why no hardware MMU isolation?

Hardware MMU isolation has a cost: every context switch flushes the TLB, every
cross-process IPC copies data.  ViOS bets that Rust's type system provides equivalent
safety guarantees at near-zero overhead.

Trade-offs accepted:
- A bug in `unsafe` kernel code can corrupt any Cell's memory — mitigated by
  minimising `unsafe` to hardware I/O only.
- Spectre-class side-channel attacks are harder to mitigate — tracked in the
  STRIDE threat model (`docs/security-model.md`).

---

## 6. What hardware does ViOS run on?

**Emulated (supported today):**
- RISC-V 64 (`qemu-system-riscv64 -machine virt`) — primary target, CI-tested
- AArch64 (`qemu-system-aarch64 -machine virt`) — secondary, boot tested
- x86_64 (`qemu-system-x86_64 -machine q35`) — secondary, boot tested

**Real hardware (planned):**
- HiFive Unmatched (RV64, post-v1.0)
- Raspberry Pi 4/5 (AArch64, post-v1.0)

---

## 7. Why no Linux compatibility?

A POSIX/Linux compatibility shim is tracked in `libs/api/src/posix.rs` for basic
syscall forwarding, but full Linux ABI compatibility is not a goal.  ViOS is a
research OS exploring a different design point; apps are written as Cells, not
POSIX processes.  For Linux app compat, see Redox or Asterinas.

---

## 8. How do I report a security issue?

**Do not open a public GitHub issue for security vulnerabilities.**

Email the maintainers at the address in `SECURITY.md` (root of repo).  We follow a
90-day responsible-disclosure window.  If no `SECURITY.md` exists yet, open a
**private** GitHub security advisory via the Security tab.

---

## 9. Where do I get help?

1. **GitHub Discussions** — Q&A category for questions; Show-and-Tell for projects
2. **GitHub Issues** — bug reports and feature requests only
3. **`docs/getting-started.md`** — step-by-step setup + common errors table
4. **Code comments** — public items have rustdoc; start with `kernel/src/main.rs`

Please search existing issues and discussions before opening a new one.

---

## 10. When is v1.0?

The v1.0 target window is **2027 H1**.  The criteria are:

- All three architectures (RV64, AArch64, x86_64) boot to shell in QEMU
- VirtIO block + input + GPU + network work without hangs
- `cargo test --workspace` passes with ≥ 80% coverage
- CI is green on every PR
- Public docs site is live

See [`docs/ROADMAP.md`](ROADMAP.md) for the full milestone breakdown.
