# Phase 08 — Multi-Arch HAL: ARM AArch64

**Effort:** 80h | **Priority:** P1 | **Status:** complete | **Blockers:** none (parallel to 03–07)

## Overview

Currently `hal/arch/arm/src/aarch64.rs` is a 38-LOC stub. Replace with a full HAL implementation so the kernel boots on `qemu-system-aarch64 -machine virt`. Builds the second supported architecture after RV64 and validates the HAL trait abstractions.

## Context Links

- `docs/04-hardware.md` — multi-arch HAL traits, VAddr/PAddr discipline
- `docs/01-core.md` — Law 3 (no pointer-size assumptions)
- RV64 implementation in `hal/arch/riscv/src/rv64/` is the reference pattern
- ARMv8-A Architecture Reference Manual (D-series, EL1/EL2/EL3 semantics)

## Key Insights

- ARMv8 has 4 exception levels (EL0=user, EL1=kernel, EL2=hypervisor, EL3=secure monitor). QEMU `virt` boots in EL1 by default with `-machine virt,virtualization=off`. We will set up EL1, run kernel in EL1, user cells in EL0.
- Page tables: 4-level (L0 → L3) with 4KB granule. TTBR0 for low VA, TTBR1 for high VA. Configure both via TCR_EL1.
- Cache/MMU enable sequence is brittle: invalidate caches → set TTBR/TCR/MAIR → `dsb sy` → `isb` → enable in SCTLR_EL1. Mis-ordering reliably reproduces "boot hang post-MMU".
- Interrupts: GIC-400 (GICv2) on QEMU virt. Distributor at 0x08000000, CPU interface at 0x08010000. SPI numbers from device tree.
- UART: PL011 at 0x09000000 on QEMU virt. Used for early println.
- Context switch: must save/restore X0–X30, SP_EL0, ELR_EL1, SPSR_EL1.

## Requirements

**Functional**
- `cargo build --release --target aarch64-unknown-none -Z build-std=core,alloc` succeeds
- Kernel boots on `qemu-system-aarch64 -machine virt -cpu cortex-a72 -kernel …`
- Prints `[ViOS] kernel boot v… (aarch64)` over PL011
- Spawns the same user_hello task from Phase 03; sees "Hi from U-mode" from EL0
- Timer IRQ via GIC drives the scheduler

**Non-functional**
- Boot time < 3s in QEMU
- No `unsafe` outside `hal/` and `kernel/src/`
- Symmetric API with RV64 HAL (same traits implemented)

## Architecture

```
EL2 (if entered) → drop to EL1 via eret
EL1 (kernel):
   ├─ Set up MAIR_EL1 (memory attribute indirection)
   ├─ Set up TCR_EL1 (TG0=4KB, T0SZ=25 → 39-bit VA)
   ├─ Build initial page tables (identity-map kernel, HHDM)
   ├─ Write TTBR0_EL1 + TTBR1_EL1
   ├─ dsb sy; isb
   ├─ Enable MMU + Cache via SCTLR_EL1 |= M|C|I
   ├─ Set up VBAR_EL1 (vector table)
   ├─ Init GIC distributor + CPU interface
   ├─ Init PL011 UART
   └─ Hand off to kernel main

EL0 (cell):
   ├─ Run on SP_EL0
   ├─ svc #0 → trap to EL1 vector → syscall dispatcher
   └─ Timer IRQ → trap to EL1 IRQ vector → scheduler
```

## Related Code Files

**Investigate:**
- `hal/arch/arm/src/aarch64.rs` — current 38-LOC stub
- `hal/arch/arm/src/aarch64/` — directory may already exist (verify)
- `hal/traits/arch/src/lib.rs` — Arch trait (RV64 implements; ARM64 will match)
- `hal/traits/paging/src/lib.rs` — PageTable trait
- `hal/traits/interrupt/src/lib.rs` — InterruptController trait
- `hal/traits/uart/src/lib.rs` — Uart trait
- `hal/traits/timer/src/lib.rs` — Timer trait

**Create (under `hal/arch/arm/src/aarch64/`):**
- `hal/arch/arm/src/aarch64/boot.rs` — `_start` assembly, EL2→EL1 transition, stack setup
- `hal/arch/arm/src/aarch64/context.rs` — `CpuContext { x0..x30, sp_el0, elr_el1, spsr_el1, ttbr0 }`
- `hal/arch/arm/src/aarch64/trap.rs` — vector table (16 entries × 0x80 bytes), synchronous/IRQ/FIQ/SError handlers
- `hal/arch/arm/src/aarch64/paging.rs` — 4-level PT, TTBR0/1 management, MAIR/TCR setup
- `hal/arch/arm/src/aarch64/gic.rs` — GIC-400 distributor + CPU interface
- `hal/arch/arm/src/aarch64/uart_pl011.rs` — PL011 UART driver
- `hal/arch/arm/src/aarch64/timer.rs` — Generic Timer (CNTPCT_EL0 + CNTPS_TVAL_EL1)
- `kernel/linker-aarch64.ld` — linker script: load base typically 0x40080000 on QEMU virt
- `scripts/run-aarch64.sh` — QEMU launch script (mirror of `run.ps1`)

**Modify:**
- `hal/arch/arm/src/lib.rs` — re-export aarch64 module behind `aarch64` feature
- `hal/arch/arm/Cargo.toml` — features `aarch64`, `aarch32`; ensure no default-on cross-deps
- `kernel/build.rs` — pick correct linker script based on `CARGO_CFG_TARGET_ARCH`
- `kernel/Cargo.toml` — `[target.'cfg(target_arch = "aarch64")'.dependencies]` block pulling in `hal-arch-arm` with `aarch64` feature
- `.cargo/config.toml` — ensure `[target.aarch64-unknown-none]` rustflags include `-C link-arg=-Tkernel/linker-aarch64.ld`

## Implementation Steps

1. **Scaffold the directory** under `hal/arch/arm/src/aarch64/` with empty stub `pub fn …` bodies for each sub-module. Make `cargo check --target aarch64-unknown-none` compile (returns 0 from stubs).
2. **PL011 UART first**: implement `uart_pl011.rs` minimally — single byte write via MMIO. This gives early println for debugging the rest. Mapped flat (identity) so writes work pre-MMU.
3. **Boot stub `boot.rs`**:
   - `.section .text.boot`
   - `_start:` clear BSS, set up early stack (in `.bss.stack`)
   - If at EL2: configure HCR_EL2, set `spsr_el2` to EL1h, `elr_el2 = el1_entry`, `eret`
   - At EL1: branch to `kmain` (Rust)
4. **Vector table `trap.rs`**: 16 entries, each 0x80 bytes. Sync EL1t, IRQ EL1t, FIQ EL1t, SError EL1t × {current SP_EL0 / current SP_ELx / lower EL using AArch64 / lower EL using AArch32}. Aligned 2048 bytes. Each entry stubs to a Rust handler.
5. **MMU `paging.rs`**:
   - Define page table entry struct: 64-bit with TYPE, AttrIdx, AP, SH, AF, nG, Output address, PXN, UXN bits
   - Build identity map for kernel image + HHDM
   - Configure MAIR_EL1: index 0 = Device-nGnRnE, index 1 = Normal WB-WA-WBRA
   - Configure TCR_EL1: TG0=00 (4KB), T0SZ=25 (39-bit VA), IPS=001 (36-bit PA) for QEMU virt
   - Write TTBR0_EL1; `dsb sy; isb`
   - Enable: `SCTLR_EL1 |= 0x1005` (M=1, C=1, I=1, SA=1); `isb`
6. **GIC `gic.rs`**:
   - Distributor enable, route SPI 30 (virtual timer) to CPU 0
   - CPU interface enable
   - Priority mask = 0xff (all priorities allowed)
   - `irq_claim()` reads GICC_IAR; `irq_complete(id)` writes GICC_EOIR
7. **Generic Timer `timer.rs`**:
   - Read frequency from CNTFRQ_EL0
   - Program CNTP_TVAL_EL1 = ticks_per_quantum
   - Enable CNTP_CTL_EL1 = 1
   - On IRQ: re-program TVAL to keep ticking
8. **Context switch `context.rs`**:
   - Save: `stp` pairs x0/x1 .. x28/x29, then x30, sp_el0, elr_el1, spsr_el1
   - Restore: reverse; `eret` returns to EL0 if SPSR.M = 0b00000
9. **Wire HAL traits**: implement `Arch`, `PageTableTrait`, `InterruptController`, `Uart`, `Timer` in their respective module files. Use the same trait surface as RV64 — if RV64 needed an extra method, mirror here.
10. **Linker script `kernel/linker-aarch64.ld`**:
    - `ENTRY(_start)`
    - `.text.boot` at 0x40080000 (QEMU virt load addr)
    - Sections: `.text`, `.rodata`, `.data`, `.bss`, `.bss.stack` (16KB)
11. **Adjust `kernel/build.rs`** to pick `linker-aarch64.ld` when target_arch is `aarch64`.
12. **`scripts/run-aarch64.sh`**:
    ```bash
    qemu-system-aarch64 -machine virt -cpu cortex-a72 -m 256M -nographic \
        -kernel target/aarch64-unknown-none/release/kernel
    ```
13. **Boot test**:
    - Run script
    - Expect: `[ViOS] kernel boot v0.2.1 (aarch64)`, then any task spawning logs
14. **Ring 3 smoke**: reuse Phase 03's `user_hello` (likely needs an aarch64 asm variant) — spawn from EL1 into EL0, expect "Hi from U-mode" via svc syscall.

## Todo List

- [x] Scaffold `hal/arch/arm/src/aarch64/` sub-modules (compile clean stubs)
- [x] Implement `uart_pl011.rs` (single-byte write)
- [x] Implement `boot.rs` (EL2→EL1, BSS clear, stack)
- [x] Implement `trap.rs` vector table with stub handlers
- [x] Implement `paging.rs` (MAIR, TCR, TTBR0, MMU enable)
- [x] Implement `gic.rs` (distributor + CPU interface)
- [x] Implement `timer.rs` (Generic Timer + IRQ)
- [x] Implement `context.rs` (save/restore + eret to EL0)
- [x] Implement Arch / PageTableTrait / InterruptController / Uart / Timer traits
- [x] Create `kernel/linker-aarch64.ld`
- [x] Update `kernel/build.rs` to pick linker by target_arch
- [x] Create `scripts/run-aarch64.sh`
- [ ] Boot test → banner appears (blocked: needs QEMU runtime)
- [ ] Ring 3 smoke test → "Hi from U-mode" via svc (blocked: needs QEMU runtime)
- [ ] Add AArch64 to CI build matrix (blocked: needs CI runner validation)

## Success Criteria

- `cargo build --release --target aarch64-unknown-none -Z build-std=core,alloc` exits 0
- `scripts/run-aarch64.sh` reaches "Hi from U-mode" within 3s
- All HAL traits implemented for AArch64 with method-parity to RV64
- CI build matrix turns AArch64 from `continue-on-error` to required

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| EL2 entry vs EL1 entry mismatch with QEMU defaults | Med | Med | Detect current EL by reading CurrentEL register; branch accordingly |
| MMU enable ordering reproduces "boot hangs after MMU on" | High | High | Strict dsb/isb discipline; reuse known-good sequence from a reference (e.g., aarch64-cpu crate) |
| Trap vector alignment (2048B) hard to enforce in Rust → use linker .balign | Med | Med | Pre-build vector as raw `.bytes` in `.S` file; link with `KEEP(.vectors)` |
| GIC-v3 vs GIC-v2 — QEMU virt defaults to GICv3 on newer QEMU | High | Med | Force `-machine virt,gic-version=2`; or implement GICv3 (more work) |
| TLS for EL0 tasks differs from RV — TPIDR_EL0 vs `tp` register | Med | Low | Abstract via Arch trait `set_user_tls()` method |
| Phase 02 CI runner may lack qemu-system-aarch64 | Low | Low | Verify `qemu-system-arm` package on ubuntu-24.04; install in CI |

## Security Considerations

- All EL0-reachable pages must have UXN bit unset and PXN bit set (cells run in EL0, kernel never executes those pages)
- EL0→EL1 syscall path: svc clears scratch registers before handler runs to prevent register-channel leaks
- PAN (Privileged Access Never) bit: keep set except inside explicit copy_from/to_user helpers

## Rollback

Code lives entirely under `hal/arch/arm/` and aarch64-only build paths; reverting the PR removes the new files and the architecture build target falls back to its prior stub. RV64 build unaffected.

## Next Steps

Pattern reused by Phase 09 (x86_64) and Phase 21 (RV32, AArch32). CI matrix promotes AArch64 from continue-on-error to required. Cells using `#[cfg(target_arch)]` for asm need an AArch64 variant; track follow-ups as `area:aarch64`.
