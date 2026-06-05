# Phase 28 — Tier 2 WASM Cells + RISC-V ePMP Foundation

**Status**: 📋 PLANNED  
**Priority**: P2  
**Target**: 2026-09-22  
**Effort**: ~5 weeks  
**Created**: 2026-06-05

---

## Goal

Add WebAssembly (WASM) as a Tier 2 Cell execution mode and lay the groundwork for RISC-V hardware memory protection (PMP). WASM cells run `.wasm` binaries inside a software-bounded linear memory sandbox, isolated from Tier 1 native cells by wasmi's bounds checker — not hardware MMU.

---

## Scope Decision: WASM first, ePMP deferred

**Research findings (2026-06-05):**

**WASM** is implementable now:
- `wasmi v1` (pure Rust, no_std + alloc, RISC-V confirmed, fuel metering, 2 deps)
- Loading path: Tier 1 Rust host ELF + `.wasm` loaded from VFS via `VfsRequest::GetFile`
- 4 custom `vi.*` WASM imports bridge to ViCell IPC
- WASM driver stub already exists at `cells/drivers/wasm/src/lib.rs`

**ePMP (full)** is blocked by M-mode architecture:
- PMP CSRs (`pmpaddr`, `pmpcfg`) are M-mode-only — S-mode kernel cannot write them
- PMP violations trap to M-mode (not S-mode) — requires fault forwarding mechanism
- Full ePMP requires a custom M-mode shim replacing/extending OpenSBI
- This is a multi-week infrastructure investment before any Cell gets isolated
- **Decision: ePMP foundation (kernel + MMIO protection only) as Phase 28-4 (optional)**

---

## Phases

| # | File | Status | Effort | Priority |
|---|------|--------|--------|----------|
| 1 | [phase-01-wasmi-integration.md](phase-01-wasmi-integration.md) | 📋 PLANNED | 5 days | P1 |
| 2 | [phase-02-host-imports.md](phase-02-host-imports.md) | 📋 PLANNED | 3 days | P1 |
| 3 | [phase-03-wasm-host-cell.md](phase-03-wasm-host-cell.md) | 📋 PLANNED | 4 days | P1 |
| 4 | [phase-04-pmp-foundation.md](phase-04-pmp-foundation.md) | 📋 PLANNED | 5 days | P2 (optional) |

**Execution order**: 1 → 2 → 3 → (4 optional, independent). Phases 1-3 are the WASM track; Phase 4 is the ePMP track.

---

## Current State (2026-06-05)

| Component | Status |
|-----------|--------|
| WASM driver crate | `cells/drivers/wasm/` exists, all `todo!()` |
| wasmi dependency | Not added |
| WASM host cell | Does not exist |
| PMP registers | Never configured anywhere in HAL/kernel |
| ELF loader | Handles PT_LOAD only; no `.wasm` section support |
| Tier 2 spec | Documented in `docs/specs/05-application.md:22-26` |

---

## Key Design Decisions

### WASM runtime: wasmi v1 (not WasmEdge, not Wasmtime)
- **WasmEdge**: C++ + libc — incompatible with `riscv64gc-unknown-none-elf`
- **Wasmtime + Pulley**: viable but requires AOT precompile + PAL implementation overhead
- **wasmi v1**: pure Rust, `no_std` + `alloc`, 2 deps (spin + wasmparser), RISC-V tested, fuel metering built-in, security-audited

### WASI: not implemented (skip entirely)
WASM cells use 4 custom `vi.*` host imports. No WASI filesystem, sockets, or clock. This matches the minimal-surface principle and keeps the WASM ABI stable.

### WASM loading: VFS read into Tier 1 host ELF
`.wasm` files stored on FAT16 at `/data/apps/`. The host cell (a Tier 1 Rust ELF) loads the `.wasm` via `VfsRequest::GetFile("/data/apps/cell.wasm")` and runs wasmi inside its own heap allocation.

### Software isolation: wasmi bounds checking (no hardware PMP for Phase 28)
WASM linear memory is a bounded `Vec<u8>`. All loads/stores are bounds-checked by wasmi's interpreter loop — same isolation model as MicroPython cells today. PMP would add hardware enforcement; deferred to Phase 28-4.

---

## Success Criteria

- [ ] A `.wasm` binary implementing a counter cell compiles and runs via the WASM host cell
- [ ] `vi.send` / `vi.recv` imports correctly bridge to `sys_send` / `sys_recv`
- [ ] Fuel metering prevents a runaway WASM cell from monopolizing the scheduler
- [ ] All 65 existing integration tests pass
- [ ] WASM cell terminates cleanly when it calls `vi.exit(0)`
