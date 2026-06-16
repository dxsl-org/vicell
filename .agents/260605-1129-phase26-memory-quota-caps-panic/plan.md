# Phase 26 — Memory Quota + ZST Capabilities + Panic Isolation

**Status**: ✅ COMPLETE (2026-06-07)  
**Priority**: P1  
**Target**: 2026-08-04  
**Effort**: ~3 weeks  
**Created**: 2026-06-05  
**Completed**: 2026-06-07 (audit confirmed all 4 phases already implemented)

---

## Goal

Three interlocking security improvements that together prevent a misbehaving Cell from crashing or starving the whole system:

1. **ZST Capability Tokens** — replace the `KernelPerms(u32)` bitfield with type-safe `Option<XxxCap>` fields; fix the `NetTx`/`NetRx` security hole (currently any Cell can call network syscalls).
2. **Per-Cell Memory Quota** — wrap `LockedHeap` in a quota-aware allocator; OOM kills only the offending Cell.
3. **Cell Fault Isolation** — replace kernel `panic!()` on unhandled exceptions with `exit_task` so a faulty Cell's crash does not halt the whole system.
4. **Audit Ring Buffer** — 256 KB SPSC in-memory log of IPC/file/net events, flushed to `/data/kernel.log`.

---

## Phases

| # | File | Status | Effort | Description |
|---|------|--------|--------|-------------|
| 1 | [phase-01-zst-caps.md](phase-01-zst-caps.md) | ✅ COMPLETE | 3 days | Replace KernelPerms bitfield with type-safe ZST cap tokens |
| 2 | [phase-02-memory-quota.md](phase-02-memory-quota.md) | ✅ COMPLETE | 4 days | Per-cell heap quota — OOM kills cell, not kernel |
| 3 | [phase-03-fault-isolation.md](phase-03-fault-isolation.md) | ✅ COMPLETE | 3 days | Trap handler kills Cell on fault instead of kernel panic |
| 4 | [phase-04-audit-ring.md](phase-04-audit-ring.md) | ✅ COMPLETE | 2 days | SPSC 256 KB audit ring buffer → /data/kernel.log |

**Execution order**: 1 → 2 → 3 → 4. Phase 1 is high-urgency (security hole). Phases 3 + 4 can run in parallel after Phase 1.

---

## Current State (2026-06-05)

| Gap | Impact | Fixed in |
|-----|--------|----------|
| `NetTx`/`NetRx` unguarded — any Cell can call | Security: Cell impersonates network | Phase 01 |
| `SpawnFromPath`/`SpawnPinned` unguarded | Security: any Cell spawns arbitrary binaries | Phase 01 |
| `KernelPerms(u32)` is an untyped bitfield | Future flag additions are error-prone | Phase 01 |
| No per-cell heap accounting | Runaway Cell exhausts global heap, halts system | Phase 02 |
| Exception in cell → `panic!()` → kernel halt | DoS: faulty Cell kills everything | Phase 03 |
| No audit trail for IPC/file/net events | Observability zero; debugging blind | Phase 04 |

---

## Key Design Decisions

### ZST caps: `pub(in crate::kernel)` constructor
```rust
// kernel/src/task/cap.rs
pub struct BlockIoCap(());
pub struct NetworkCap(());
pub struct SpawnCap(());

impl BlockIoCap {
    // Kernel-only constructor — Cells are separate Rust crates, they cannot name this path.
    pub(in crate::kernel) fn new() -> Self { Self(()) }
}
```
`Option<ZST>` = 1 byte (niche optimization). Three caps = 3 bytes vs `KernelPerms(u32)` = 4 bytes. Net smaller.

### Memory quota: `CURRENT_CELL_ID` atomic + wrapper allocator
The `linked_list_allocator` exposes no metrics API — must wrap it at the `GlobalAlloc` trait level. A global `AtomicUsize CURRENT_CELL_ID` is set by the scheduler on every context switch. The wrapper reads it and charges the allocation to the correct Cell's quota entry.

### Panic isolation: hardware trap as the Cell boundary
`panic = "abort"` is set — `catch_unwind` is impossible without switching the panic runtime (~25 KB overhead, not worth it). The correct boundary is the RISC-V trap handler: exceptions (illegal instruction, page fault) in a Cell context call `exit_task(current_cell_id)` + `yield_cpu()` instead of `panic!()`.

### Audit ring: hand-rolled SPSC (no dep)
~60 lines. `[u64 mtime][u8 event_type][u8 len][payload]` format. `AtomicUsize` head/tail with `Release`/`Acquire` ordering. Power-of-two size for branchless modulo.

---

## ⚠️ Law 1 Gate

Adding `BlockIoCap`, `NetworkCap`, `SpawnCap` to `libs/api/` is **not required** — these are kernel-internal types. Cell ABI is unchanged. Law 1 confirmation NOT needed for Phase 26.

However, removing `KernelPerms` from TCB is an internal kernel struct change (not ABI). No confirmation required.

---

## Success Criteria

- [ ] Any Cell calling `NetTx`/`NetRx` without `NetworkCap` gets `PermissionDenied`
- [ ] Any Cell calling `SpawnFromPath` without `SpawnCap` gets `PermissionDenied`
- [ ] Cell that allocates past its 4 MiB quota is terminated; kernel keeps running
- [ ] Cell that executes an illegal instruction is terminated; shell resumes
- [ ] `/data/kernel.log` grows with IPC/file/net events during normal operation
- [ ] `cargo test --all --release` passes (all 65 existing tests)
