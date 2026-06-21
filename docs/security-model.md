# ViCell Security Model

**Version:** v0.2.3-dev | **Updated:** 2026-06-21

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
- Long-term: CHERIoT RISC-V hardware capabilities — see "Hardware Isolation Roadmap" section below

**Do NOT use ViCell to run untrusted third-party code until Tier 3 VM is implemented.**

> **Full analysis:** [research/research-hardware-isolation.md](research/research-hardware-isolation.md) — covers the
> full menu of hardware supplements (CFI, MPK/PKS, MPU/PMP, RISC-V WorldGuard/Smmtt, IOMMU/IOPMP, confidential
> computing, CHERI), each rated against the SAS "no-TLB-flush-per-Cell-switch" criterion, plus peer-OS prior art
> (Tock, Hubris, RedLeaf, Theseus, Singularity, CheriOS) and a severity-ranked gap list.

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

### DMA Isolation Absent — IOMMU in Passthrough Mode
**Severity: Critical (real hardware with DMA-capable peripherals)**

The RISC-V IOMMU and x86 VT-d shipped in Track B run in **passthrough mode** — RISC-V via `DDTP.MODE=1`
(bare; translation disabled), x86 via VT-d with translation *enabled* (`TES`) but every one of the 256 BDFs
mapped to a `TT=0b10` passthrough context entry in a single shared domain. Both yield IOVA==PA with no
permission table, and `iommu::map_dma()` is a literal identity no-op (`kernel/src/task/drivers/iommu.rs`) —
functionally equivalent to having no IOMMU. In a SAS, a **Cell *is* the
driver**: it owns the device MMIO and programs the DMA descriptor ring directly, with no kernel intermediary.
A compromised (or buggy) Cell that holds a DMA-capable peripheral (NIC, NVMe, GPU, USB host, on-chip DMA
engine) can read or write **any** physical address — kernel page tables, scheduler/Cell metadata, other Cells'
stacks — **without a single line of `unsafe`**, purely via MMIO writes it is legitimately permitted to issue.
This defeats LBI and every CPU-side memory protection at once. The blast radius equals a Linux *kernel driver*
bug, not a user-space exploit. (Thunderclap, NDSS 2019, bypassed the IOMMU of macOS/Linux/FreeBSD even when
*enabled* — ViCell has not enabled translation at all.)

**Key distinction**: MMIO ownership ≠ DMA authorization. The Resource Registry enforces exclusive MMIO
ownership, but holding NIC MMIO implies DMA capability while holding UART MMIO does not. The kernel must track
DMA capability **separately** and install per-device, per-Cell IOMMU/IOPMP translation entries.

**Planned**: Switch IOMMU from passthrough → translate mode with per-device IOVA→PA tables; add
`sys_grant_dma(device, phys, size)` mapping only granted pages; RISC-V **IOPMP** (bus-side) for on-chip DMA
controllers that bypass the SMMU. Must be resolved before any Cell is granted a real DMA-capable peripheral on
hardware. See [research/research-hardware-isolation.md](research/research-hardware-isolation.md) §3.

### Forward-Edge CFI Absent (BTI / CET-IBT)
**Severity: High (prerequisite for MPK)**

Spatial memory protection does not stop a corrupted indirect branch from jumping anywhere in the SAS. ViCell
plans PAC (ARM) but PAC only covers the **backward edge** (return addresses) — forward-edge JOP/COP is open
without **BTI** (ARM) or **CET-IBT** (x86). Critically, **MPK/PKU is not a security boundary without CFI**:
`WRPKRU` is an unprivileged instruction, so any JOP gadget reaching an unsanctioned `WRPKRU`/`XRSTOR` grants
the attacker every protection key (ERIM, USENIX Sec 2019; PKU Pitfalls, USENIX Sec 2020 — 10 working bypasses).
Safe Rust Cells neutralize most of this, but C FFI (mlibc, DOOM), Lua dispatch, and unsafe kernel code
re-expand the gadget surface to the whole image.

**Planned**: Compile Cells + kernel with `+bti,+pac-ret` (ARM) / `CONFIG_X86_KERNEL_IBT` + CET Shadow Stack
(x86); make CFI a hard prerequisite before enabling any MPK domain. See
[research/research-hardware-isolation.md](research/research-hardware-isolation.md) §2.

### No Audit Log
**Severity: Medium**

Cell actions (IPC sends, file writes, network connects) are not persistently logged. Forensic analysis after an incident is impossible.

**Planned**: 256 KB kernel ring buffer, flushed to `/data/kernel.log` on shutdown (Phase 26).

### Capability Token System — Implementation Gap
**Severity: Medium**

Spec (01-core.md) describes unforgeable Zero-Sized Type capability tokens. Current implementation uses an ELF
manifest of one `flags: u8` (8 boolean flags, **all used**) plus a `can_block_io` TCB flag — coarse, granted
all-at-spawn, with **no scoping, no delegation, no revocation, no user/operator consent**. This is effectively
the **Android pre-6.0 install-time permission model**, and violates all four capability-OS invariants (no
ambient authority · explicit delegation · monotonic downgrade · revocable).

**Planned** (see [research/research-cell-security-permissions.md](research/research-cell-security-permissions.md)
for the full design + capability-OS / mobile-OS references): evolve through (1) **parameterized capabilities**
(`__ViCell_cap_args` ELF section — e.g. "GPIO pins 14-17" not "all GPIO"; additive, no Law 1 bump), (2)
**spawn-time intersection** (a Cell can only delegate caps it holds — kills confused-deputy), (3) **runtime
revocation** (`CapHandle` + `sys_cap_revoke`), (4) **operator-signed policy** for headless G1 fleets (consent =
signed policy, NOT a dialog — see the headless-robot caveat) and an optional **TCC-style consent-broker Cell**
for G2 HMI (sensitive caps only). Hard invariant: the manifest is a **ceiling, not a floor**, and **only the
kernel enforces** (consent feeds the syscall-boundary check). LBI already closes the TCC "permission-laundering
via code injection" hole that produced repeated macOS/iOS TCC CVEs.

### Boot Trust Chain + Attestation — Absent
**Severity: Medium (High for fleet deployment)**

ViCell has no secure boot, no measured boot, no device attestation, and no sealed storage. Cell binary signing
(Ed25519/P-256) is planned but unimplemented. A robot fleet cannot cryptographically prove a device runs
unmodified ViCell (vs tampered firmware with a cloned identity), and secrets are not bound to a measured boot
state.

**Planned** (see [research/research-cell-security-permissions.md](research/research-cell-security-permissions.md)
§3): a TPM-free **DICE/RIoT** layered chain (`CDI_n = HKDF(CDI_{n-1}, HASH(layer_n))`), per-Cell **SHA-256
measurement** at `spawn_from_path()` extended into a kernel measurement log (Linux IMA model), **remote
attestation** via EAT tokens (RFC 9711) + RATS (RFC 9334) verified by ARM Veraison fleet-side, and **sealed
storage** with the AEAD key held in the **Silo** (closes the CDI-in-RAM exposure). Hardware root of trust:
**OpenTitan** (open-source RISC-V) is the natural backing for the existing Silo abstraction.

## Hardware Isolation Roadmap

> **Full menu + status:** [research/research-hardware-isolation.md](research/research-hardware-isolation.md).
> This section keeps only the CHERI sub-roadmap; CFI / MPK-PKS / MPU-PMP / WorldGuard-Smmtt / IOMMU-IOPMP /
> confidential-computing supplements live in the research doc and the project roadmap §G.

### CHERI sub-roadmap

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
| **CHERIoT-IBEX** (lowRISC/Microsoft) | ✅ Sonata FPGA (~$412); SCI ICENI silicon Early-Access 2025; Rust no_std fork active (cập nhật hằng tuần từ 2/2026) | RV32E (embedded) |
| **Morello** (ARM) | ❌ **ARM tuyên bố KHAI TỬ** — không sản phẩm, không kế thừa có tên (eval ~20-35% overhead) | AArch64 CHERI (EoL) |
| **RISC-V "Zcheri" extension** | ❌ **Chưa ratify** (target đầu 2026, đã trượt) | RV32CH/RV64CH |
| **CHERI-RISC-V RV64** (Cambridge / COSMIC) | 🔶 FPGA only; COSMIC nhắm secure-enclave 3/2028, chưa tape-out | RV64 full CHERI |

> **Thực tế (2026)**: CHERI cho RV64 (target chính của ViCell) **chưa có silicon, chưa có Rust target, ISA chưa ratify**
> — KHÔNG khả thi cho 2026-Q4; realistic 2028-2030. ARM Morello đã bị khai tử. **CHERIoT-IBEX là RV32E** và là path
> duy nhất chín muồi — phù hợp **ViCell-Nano** profile (embedded robots). Compartment switch đo được 209-452 cycle
> (nhanh hơn null syscall, SOSP 2025).

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
| HW — spatial _(roadmap)_ | MPU/PMP (embedded C-tier), MPK/PKS (x86 tier domains), MTE (ARM UAF hardening) |
| HW — control-flow _(roadmap)_ | BTI+PAC (ARM), CET IBT+Shadow Stack (x86), Zicfilp/Zicfiss (RISC-V) — **prerequisite for MPK** |
| HW — DMA _(roadmap)_ | IOMMU/SMMU translate mode + IOPMP; per-Cell `sys_grant_dma` (**not** MMIO ownership) |
| HW — VM-grade _(roadmap)_ | Stage-2/EPT (Tier 3); TDX/SEV-SNP/ARM CCA for attested multi-tenant |

> Hardware layers are rated against the SAS "no-TLB-flush-per-switch" criterion in
> [research/research-hardware-isolation.md](research/research-hardware-isolation.md).

## Security Contacts

For vulnerability reports, open a GitHub Issue with label `security`.
Critical issues (RCE, privilege escalation): email directly (see SECURITY.md).
