# Phase 04 — RISC-V PMP Foundation (Optional)

**Status**: 📋 PLANNED  
**Priority**: P2 (optional — independent of Phases 01-03)  
**Effort**: 5 days

---

## ⚠️ Architecture Warning — Read Before Implementing

**PMP registers are M-mode-only.** ViCell's kernel runs in S-mode under OpenSBI. Direct CSR writes to `pmpaddr` / `pmpcfg` from the kernel will raise an Illegal Instruction trap. This phase requires one of:

**Option A (recommended)**: A custom M-mode shim that runs before OpenSBI and configures PMP, then drops to S-mode. This shim sits between the bootloader and OpenSBI, or replaces OpenSBI as the M-mode firmware.

**Option B**: A custom SBI extension (OpenSBI vendor extension) that the S-mode kernel calls via ecall to request PMP reconfiguration. Requires patching OpenSBI.

**Option C**: Rely on OpenSBI's existing `fw_dynamic` platform layer to configure PMP at boot from a static table defined in the firmware binary. No runtime reconfiguration from S-mode.

**This phase implements Option C** — static boot-time PMP configuration only, no per-Cell runtime reconfiguration. Runtime per-Cell PMP enforcement deferred to Phase 32 (SMP + full ePMP).

---

## Context Links

- RISC-V PMP spec: https://docs.riscv.org/reference/isa/priv/smepmp.html
- OpenSBI platform layer: `opensbi/lib/platform/`
- HAL boot entry: `hal/arch/riscv/src/rv64/boot.rs`
- Linker script: `kernel/linker.ld` — kernel PA ranges
- QEMU virt PMP count: 16 entries (2 consumed by OpenSBI → 14 free)

---

## Overview

Phase 04 adds **static boot-time PMP protection** for the kernel's own regions. This is not per-Cell isolation — it is the baseline protection that makes it harder for a rogue Cell to corrupt kernel code or data.

After Phase 04, the kernel's text region is R-X (no write), data region is R-W (no execute). MMIO ranges are marked accessible. This matches the W^X principle already encoded in the linker script.

Per-Cell PMP (one NAPOT entry per Cell) requires runtime reconfiguration from M-mode and is explicitly out of scope here.

---

## PMP Entry Layout (14 free entries after OpenSBI)

| Entry # | Region | Config | Notes |
|---------|--------|--------|-------|
| 2 | Kernel `.text` + `.rodata` (`0x80200000 – __bss_start`) | R-X, NAPOT | No write to kernel code |
| 3 | Kernel `.data` + `.bss` + stack | R-W, NAPOT | No execute in data |
| 4 | VirtIO MMIO range (`0x1000_0000 – 0x1001_0000`) | R-W, NAPOT | Device access |
| 5 | PLIC (`0x0C00_0000 – 0x1000_0000`) | R-W, NAPOT | Interrupt controller |
| 6 | CLINT (`0x0200_0000 – 0x0201_0000`) | R-W, NAPOT | Timer/IPI |
| 7–14 | Reserved for future per-Cell entries | OFF | Available for Phase 32 |

Note: All addresses use NAPOT encoding. NAPOT granularity constraint requires region size = power of two and base aligned to region size. The kernel text/data regions from `linker.ld` may need padding to meet NAPOT alignment.

---

## Related Code Files

### Create
- `hal/arch/riscv/src/common/pmp.rs` — PMP CSR write helpers and region config

### Modify
- `hal/arch/riscv/src/rv64.rs` — call `pmp::init_static_regions()` in M-mode boot path (if accessible)
- `hal/arch/riscv/src/rv64/boot.rs` — or add PMP init in assembly before S-mode handoff

---

## Implementation Steps

### ⚠️ CRITICAL DESIGN CONSTRAINT

**`csrw pmpaddr*` / `csrw pmpcfg*` are M-mode-only CSRs.** Writing them from S-mode raises Illegal Instruction. ViCell uses `-bios default` (OpenSBI as M-mode firmware, kernel in S-mode). **No inline asm can configure PMP from the S-mode kernel directly.**

Option C (chosen for Phase 04) means: PMP is configured by OpenSBI's firmware using a region table compiled into the firmware binary. The S-mode kernel does NOT write PMP CSRs. The kernel only reads the resulting protection via PMP fault traps (which go to M-mode, not S-mode — see Phase 04 risk table).

### Step 1 — Define PMP region table (for future M-mode shim)

This file documents the intended region layout for when a custom M-mode firmware is available. No kernel code calls these at runtime in Phase 04.

```rust
//! RISC-V PMP region definitions for static boot-time protection.
//!
//! These constants define the desired PMP layout.  They are consumed by
//! a custom M-mode firmware (not by the S-mode kernel directly — PMP CSRs
//! are M-mode-only and cannot be written from S-mode).
//!
//! NAPOT formula: pmpaddr = (base >> 2) | (size/8 - 1)
//! Both `(base >> 2) | ((size/2 - 1) >> 2)` and `(base >> 2) | (size/8 - 1)`
//! are arithmetically equivalent for power-of-two sizes.

pub mod perm {
    pub const R:      u8 = 0b001;
    pub const W:      u8 = 0b010;
    pub const X:      u8 = 0b100;
    pub const RW:     u8 = R | W;
    pub const RX:     u8 = R | X;
    pub const A_NAPOT: u8 = 0b11 << 3;
    pub const L:      u8 = 1 << 7; // locked
}

/// A PMP region descriptor consumed by the M-mode firmware table.
pub struct PmpRegion {
    pub base:  usize,
    pub size:  usize, // must be power-of-two, ≥ 8
    pub perms: u8,
}

/// Compute the NAPOT pmpaddr value for a region.
pub const fn napot_addr(base: usize, size: usize) -> usize {
    (base >> 2) | (size / 8 - 1)
}

/// Intended kernel protection regions (entries 2-6; 0-1 are OpenSBI's).
pub const KERNEL_PMP_REGIONS: &[PmpRegion] = &[
    PmpRegion { base: 0x8020_0000, size: 4*1024*1024, perms: perm::RX  | perm::A_NAPOT | perm::L },
    PmpRegion { base: 0x8060_0000, size: 4*1024*1024, perms: perm::RW  | perm::A_NAPOT | perm::L },
    PmpRegion { base: 0x1000_0000, size: 65536,        perms: perm::RW  | perm::A_NAPOT },
    PmpRegion { base: 0x0C00_0000, size: 16*1024*1024, perms: perm::RW  | perm::A_NAPOT },
    PmpRegion { base: 0x0200_0000, size: 65536,        perms: perm::RW  | perm::A_NAPOT },
];
```

### Step 2 — Verify OpenSBI configures PMP correctly

Check QEMU serial output for OpenSBI's PMP log lines:
```
Boot HART PMP Count         : 16
Boot HART PMP Granularity   : 4 bytes
```

OpenSBI by default configures PMP0 (firmware deny) and PMP1 (all-access). Kernel `.text` is NOT protected yet. Phase 04's actual protection goal requires either patching OpenSBI's platform configuration or building a custom M-mode shim.

---

## Todo List

- [ ] Create `hal/arch/riscv/src/common/pmp.rs` (set_napot + init_static_regions)
- [ ] Add `pub mod pmp;` to `hal/arch/riscv/src/common.rs`
- [ ] Determine if ViCell boots in M-mode (check OpenSBI `-bios default` handoff)
- [ ] If M-mode accessible: add call to `pmp::init_static_regions()` in boot assembly
- [ ] If M-mode NOT accessible: document blocker and defer to custom firmware
- [ ] Integration test: verify kernel text is read-only (WASM cell attempt to write fails)

---

## Success Criteria

- [ ] PMP entries 2-6 configured with correct permissions at boot
- [ ] `pmpcfg0/pmpcfg2` CSR values match expected layout (verified via log)
- [ ] Kernel `.text` region is RX (a rogue pointer write to kernel code causes access fault)
- [ ] Kernel data region is RW (no execute)

---

## Risk Assessment

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| ViCell boots entirely in S-mode (no M-mode access) | **High** | Phase 04 is blocked; defer until M-mode shim is built |
| NAPOT alignment: kernel regions not power-of-two aligned | Medium | Pad kernel regions in linker.ld to next power-of-two |
| PMP fault traps to M-mode (not S-mode) — kernel never sees it | Confirmed | Accept this for static protection; dynamic fault handling deferred |
| Locked entries cannot be reconfigured — wrong config = reboot required | High | Test NAPOT calculation offline before writing to hardware |
