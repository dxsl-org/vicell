# ViCell Security Model

**Version:** v0.2.1-dev | **Updated:** 2026-05-29

## Design Philosophy

ViCell uses a **Cellular Single Address Space (SAS)** model with
Language-Based Isolation (LBI) via Rust's type system.  Traditional OS
security relies on hardware MMU separation between processes; ViCell instead
relies on:

1. **Rust ownership + borrow checker** — prevents spatial/temporal memory bugs
2. **Capability tokens (`CapId`)** — unforgeable, kernel-managed access rights
3. **`#![forbid(unsafe_code)]` on Cells** — enforced by `cargo-geiger` in CI
4. **VFS access through capabilities** — no direct file-descriptor integers

## STRIDE Threat Model

### Spoofing
| Threat | Mitigation | Status |
|--------|-----------|--------|
| Cell forges another Cell's CellId in IPC | Kernel verifies sender ID from TCB on every message; user cannot inject arbitrary sender values | ✅ Mitigated |
| Cell constructs a valid CapId by guessing | CapIds are kernel-assigned opaque u64 values; 64-bit ID space makes guessing infeasible | ✅ Mitigated |
| Malformed ELF binary spawns as a different Cell | ELF header validated before execution; Cell registry assigns IDs monotonically | ✅ Mitigated |

### Tampering
| Threat | Mitigation | Status |
|--------|-----------|--------|
| Cell writes to another Cell's memory | SAS + Rust ownership; no `unsafe` in cells/ | ✅ Mitigated |
| Cell modifies a revoked capability | Kernel removes CapId from table on Close; subsequent ops return PermissionDenied | ✅ Mitigated |
| Attacker modifies disk image to inject malicious ELF | ELF is checksummed at load time (TODO: Phase 12 adds SHA256 verification) | 🔶 Partial |

### Repudiation
| Threat | Mitigation | Status |
|--------|-----------|--------|
| Cell claims it did not send an IPC message | Sender ID in TCB is set by kernel on message delivery; cannot be forged | ✅ Mitigated |
| Audit log missing | No audit log yet; planned for v1.x | ❌ Deferred |

### Information Disclosure
| Threat | Mitigation | Status |
|--------|-----------|--------|
| Kernel leaks pointers to user-mode | Kernel zeroes new frames before mapping; TrapFrame zeroes scratch regs on EL0 return | ✅ Mitigated |
| Spectre / Meltdown side-channel | Known limitation; not mitigated in v1.0.  See `known_limitations` below | ⚠️ Known |
| File content readable without capability | VFS returns data only to cells holding a valid `CapId` with READ permission | ✅ Mitigated |

### Denial of Service
| Threat | Mitigation | Status |
|--------|-----------|--------|
| Cell allocates unbounded memory | Frame allocator has a hard cap (total usable RAM); OOM kills the cell | ✅ Mitigated |
| Cell floods IPC queue | Message queue is bounded; sender blocks when full (future Phase 20) | 🔶 Partial |
| Lua/MicroPython script infinite loop | Cell exit triggered by kernel timeout (future scheduler enhancement) | ❌ Deferred |

### Elevation of Privilege
| Threat | Mitigation | Status |
|--------|-----------|--------|
| Cell executes privileged instruction (e.g., `wfi`) | Cells run in EL0/Ring3; trap dispatched to kernel | ✅ Mitigated |
| `#[allow(unsafe_code)]` in a Cell | `cargo-geiger` CI gate fails if any Cell contains `unsafe`; zero-tolerance policy | ✅ Mitigated |
| Malformed syscall arguments overflow kernel buffers | All syscall arg lengths validated via `validate_user_buf` before dereference | ✅ Mitigated |

## Known Architecture Risks

### Spectre v1/v2 — SAS Worst-Case Scenario
**Severity: Critical (research/trusted-environment only)**

SAS is the worst-case environment for Spectre attacks. In a traditional OS, Spectre leaks within a single process boundary. In ViCell SAS, a compromised Tier 1 cell can speculatively read any memory in the entire system — including kernel heap, crypto keys, and other cells.

**Current status**: No mitigation. ViCell v1.0 requires all Tier 1 cells to be trusted (signed, first-party code).

**Mitigations planned**:
- Short-term: Document "trusted cells only" constraint explicitly (done here)
- Medium-term: Tier 3 VM isolation for untrusted code (hardware page tables per VM)
- Long-term: CHERIoT RISC-V hardware capabilities — see "CHERI Integration Roadmap" section below

**Do NOT use ViCell to run untrusted third-party code until Tier 3 VM is implemented.**

> **Isolation strategy decision (2026-06-05):** per-Cell **SATP** isolation at Tier 1 is
> **explicitly NOT pursued**. PMP is M-mode-only (unreachable from ViCell's S-mode without
> custom firmware) and sPMP is unratified; per-cell SATP would break Tier 1 zero-copy IPC.
> Hardware isolation is delivered by **Tier 3 Stage-2 paging (per-VM)**, and untrusted code
> is confined to Tier 2 (WASM) / Tier 3. The Tier 1 "signed cells only" guarantee depends on
> **Ed25519 signing + a secure-boot loader gate** (currently spec-only — "trusted" is today
> approximated by the `/bin/` path prefix). See [specs/12-reliability.md](specs/12-reliability.md) §2.

### KASLR Absent
**Severity: High**

Kernel loads at a fixed virtual address. An attacker with any code execution can immediately locate kernel symbols without brute-forcing.

**Planned**: KASLR via Limine boot randomization — estimated 3 days to implement, deferred to Phase 24.

### No Audit Log
**Severity: Medium**

Cell actions (IPC sends, file writes, network connects) are not persistently logged. Forensic analysis after an incident is impossible.

**Planned**: 256 KB kernel ring buffer, flushed to `/data/kernel.log` on shutdown (Phase 26).

### Capability Token System — Implementation Gap
**Severity: Medium**

Spec (01-core.md) describes unforgeable Zero-Sized Type capability tokens. Current implementation uses a `can_block_io` TCB flag — a simpler mechanism that works for current cell count but does not scale to arbitrary capability types.

**Planned**: Full ZST capability token system when Cell count exceeds 20.

## CHERI Integration Roadmap

**CHERIoT** (Capability Hardware Extension RISC-V for IoT) là extension RISC-V cung cấp **hardware-enforced pointer bounds** — kết hợp hoàn hảo với Rust LBI của ViCell.

### Tại sao CHERI quan trọng với ViCell

| Cơ chế | Rust LBI (hiện tại) | CHERI + Rust LBI |
|--------|---------------------|-----------------|
| Bounds checking | Compile-time only | Compile-time **+** hardware runtime |
| `unsafe` blocks | Không được kiểm soát bounds | Hardware trap nếu pointer ra ngoài bounds |
| Spectre gadgets | Không mitigate | Capability bounds giới hạn speculative access |
| Pointer forgery | Compiler ngăn trong safe Rust | Hardware ngăn kể cả trong `unsafe` |
| Use-after-free trong HAL | Phụ thuộc code review | Hardware trap ngay lập tức |

**Kết luận**: Rust LBI + CHERI = defense-in-depth thực sự. Rust bắt lỗi lúc compile; CHERI bắt lỗi còn sót lại lúc runtime — kể cả trong kernel `unsafe` blocks.

### Silicon availability (2026)

| Platform | Status | CHERI Type |
|----------|--------|-----------|
| **CHERIoT-IBEX** (lowRISC/Microsoft) | ✅ Production silicon, FPGA | RV32 (embedded) |
| **Sonata dev board** (lowRISC) | ✅ Có thể mua ngay | CHERIoT-IBEX RV32 |
| **Morello** (ARM) | ✅ Limited hardware | AArch64 CHERI |
| **CHERI-RISC-V RV64** (Cambridge) | 🔶 Research, FPGA only | RV64 full CHERI |
| **Standard RISC-V RV64 với CHERI** | ❌ Chưa có silicon | — |

> **Thực tế**: CHERI cho RV64 (target chính của ViCell) chưa có production silicon. CHERIoT-IBEX là RV32 — phù hợp cho **ViCell-Nano** profile (embedded robots, constrained devices).

### Integration Path với ViCell (Phase 31)

```
Bước 1: HAL arch mới
  cells/hal/arch/cheriot32/      # CHERIoT-IBEX RV32 target
  - Capability registers thay thế VAddr/PAddr
  - Memory tagging qua hardware capability table

Bước 2: libs/types thay đổi
  #[cfg(feature = "cheri")]
  pub type VAddr = CheriCapability;  // hardware capability
  pub type PAddr = CheriCapability;

Bước 3: Rust toolchain
  - Dùng CHERIoT-Platform/rust fork (rustc CHERI support)
  - Target: riscv32cheriot-unknown-unknown
  - Không cần thay đổi Tier 1 cell code (Rust LBI vẫn hoạt động)

Bước 4: Kernel unsafe blocks
  - Mỗi unsafe block tự động được hardware bounds-check
  - SAS attack surface giảm từ "toàn bộ address space"
    xuống "chỉ các capabilities được cấp phép"
```

### Prerequisites (Phase 31)

- [ ] Mua Sonata development board (CHERIoT-IBEX, ~$50)
- [ ] Xác nhận CHERIoT-Platform/rust build cho no_std ViCell target
- [ ] Thiết kế `feature = "cheri"` flag trong libs/types không breaking existing RV64 code
- [ ] Benchmark: overhead của CHERI bounds check vs. phần mềm Rust LBI

**Target**: Phase 31 (2026-Q4) cho ViCell-Nano profile trên Sonata board.

---

## Known Limitations

1. **Spectre v1/v2:** The SAS model means kernel and all Cells share a
   virtual address space.  Spectre-class microarchitectural leakage is
   inherent.  Mitigation (retpoline, IBRS, CSR flushing) is deferred to
   Phase 12 hardening.

2. **No KASLR:** Kernel is loaded at a fixed address by Limine.  Planned
   for v1.x.

3. **Trusted Cells:** All installed Cells are fully trusted.  There is no
   sandbox for untrusted Cells in v1.0.  See Phase 23 for community
   submission review gates.

4. **No audit log:** Cell actions are not persistently logged.

## Defense in Depth

| Layer | Mechanism |
|-------|-----------|
| Language | `#![forbid(unsafe_code)]` on all Cell crates |
| Compile-time | Rust ownership, borrow checker, lifetimes |
| Kernel | Capability table, syscall argument validation, frame zeroing |
| CI | `cargo-geiger`, `cargo-audit`, `cargo-deny` on every PR |
| Fuzzing | Weekly libFuzzer harnesses on ELF parser + VFS path validator |

## Security Contacts

For vulnerability reports, open a GitHub Issue with label `security`.
Critical issues (RCE, privilege escalation): email directly (see SECURITY.md).
