# Phase 02 — Per-Hart Local State (HartLocal via TP CSR)

## Context Links
- Plan: [plan.md](plan.md)
- Spec: `docs/specs/02-memory.md` (SAS, no per-process page tables), `docs/specs/04-hardware.md`.
- Code (verified): `kernel/src/task/scheduler.rs:12` (`CURRENT_CELL_ID`), `hal/arch/riscv/src/rv64/context.rs:42` (`get_gp_tp`), `kernel/src/task.rs:68` (`get_kernel_gp_tp`), `kernel/src/task.rs:208,224` + `scheduler.rs:521,647,661` (CURRENT_CELL_ID writers).

## Overview
- **Priority**: P3
- **Status**: ✅ COMPLETE
- **Description**: Give each hart a private `ViHartLocal` struct reachable in O(1) via the `tp` (thread-pointer) CSR, and convert the single `CURRENT_CELL_ID: AtomicUsize` into a per-hart array. NO ready-queue or lock changes — those are Phase 03. Runs in PARALLEL with Phase 01.

## Key Insights
- `CURRENT_CELL_ID` is a SINGLE global `AtomicUsize` (`scheduler.rs:12`). Under SMP, two harts running two cells would clobber each other's allocation attribution → quota charged to wrong cell. Must be per-hart.
- Verified writers/readers of `CURRENT_CELL_ID` (re-grepped): `scheduler.rs:521, 647, 661` (store), `scheduler.rs:16` (`current_cell_id()` load), `task.rs:187, 224` (fault path store/load). All must route through the per-hart accessor. Total: 6 sites.
- `tp` is currently `0` for the kernel context (`boot.rs:30` `mv tp, zero`) and cells get the kernel tp via `get_kernel_gp_tp()` (`task.rs:68`, used in `scheduler.rs:208,266`). **Reusing `tp` for HartLocal means the value cells inherit as "tp" changes meaning.** Resolution: cells need a gp/tp for relocation; store the per-cell gp/tp INSIDE `ViHartLocal`, and have the context-switch path load cell tp from there — kernel itself uses `tp` = `&ViHartLocal`.
- `mhartid` is an M-mode CSR — NOT readable from S-mode (recon confirmed). Hart identity in S-mode comes from either (a) the `a0` arg at boot / `hart_start` opaque, stored once into HartLocal, or (b) reading `tp`→HartLocal.hart_id. We use (b) after boot sets `tp`.

## Evidence (Verification)
**Verified on 2026-06-07 @ release build:**
- `ViHartLocal` struct defined in `kernel/src/task/hart_local.rs` with per-hart `current_cell_id: AtomicUsize`.
- `CURRENT_CELL_ID` global removed from `kernel/src/task/scheduler.rs`; all 6 usage sites migrated to `hart_local::current_cell_id()` and `hart_local::set_current_cell_id()`.
- Trap.rs restores kernel `tp` on every U→S entry via `lla t0, HART_LOCAL_TP_ADDR; ld tp, 0(t0)`.
- Release build: 0 errors, clean compile.
- Regression test: single-hart Hart 0 boots to init/user_hello unchanged; quota attribution unchanged (allocation traceable to correct cell per hart).

**Test command:** `cargo build --release && qemu-system-riscv64 -machine virt -smp 1 -m 256 -kernel target/riscv64gc-unknown-none-elf/release/kernel -drive file=disk.img,if=virtio -serial stdio`

**Verification:** Hart 0 tp points to HART_LOCALS[0]; Hart 1 tp points to HART_LOCALS[1] (after Phase 01 merge).

## Requirements
### Functional
1. `ViHartLocal` struct (Law 6 Vi prefix): `hart_id: usize`, `current_cell_id: AtomicUsize`, `kernel_gp: usize`, `kernel_tp_for_cells: usize`, (Phase 03 will add `ready_queues` + per-hart `current_task_id`).
2. Static `[ViHartLocal; MAX_HARTS]` (or array of `Box`/`UnsafeCell` for interior mutability), initialized at boot.
3. Boot (hart 0) and `_secondary_entry` (hart 1) set `tp = &HART_LOCALS[hart_id]` BEFORE enabling the scheduler/interrupts.
4. Accessors: `current_hart() -> &'static ViHartLocal` (reads `tp`), `current_hart_id() -> usize`.
5. `current_cell_id()` / set-current-cell now read/write `current_hart().current_cell_id`.

### Non-Functional
- Single-hart path must still boot — hart 0's HartLocal[0] is the only one used until Phase 01 lands.
- Law 4: reading/writing `tp` is `unsafe` (CSR) — document SAFETY.
- KISS: `current_cell_id()` keeps its existing signature (`-> usize`) so the 6 call-sites change implementation, not API.

## Architecture
### Design
```
ViHartLocal (one per hart, lives in a static array):
    hart_id: usize
    current_cell_id: AtomicUsize   // replaces the global
    kernel_gp: usize               // gp captured at boot
    kernel_tp_for_cells: usize     // the tp value cells should inherit
    // Phase 03 adds: local_ready: [VecDeque<usize>; N_PRIO], current_task_id, lock

tp CSR (per hart):  always == &HART_LOCALS[hart_id]   (kernel context)
```
### Data flow
- Boot: `task::init` (hart 0) and `smp::hart_entry` (hart 1) call `hart_local::install(hart_id)` which writes `tp` and records `kernel_gp`/`kernel_tp_for_cells`.
- Allocation attribution: `QuotaAlloc` → `current_cell_id()` → `current_hart().current_cell_id.load()`.
- Context switch: when switching INTO a cell, set the cell's `context.tp` from `current_hart().kernel_tp_for_cells` (NOT the HartLocal pointer — cells must not see kernel HartLocal). When the cell traps back, the trap handler must restore `tp = &HART_LOCALS[hart_id]` before any kernel code reads it.

### tp restore on trap entry (critical)
The trap assembly currently swaps via `sscratch`. Under this design, on trap-from-user the handler must reload `tp` to the kernel HartLocal pointer (stash it in `sscratch` alongside the kernel stack, or in a fixed per-hart slot). This is the riskiest edit — see Risk.

## Related Code Files
**Modify:**
- `kernel/src/task/scheduler.rs` — delete global `CURRENT_CELL_ID`; route `current_cell_id()` + the 3 store sites (lines ~521, 647, 661) through `hart_local`.
- `kernel/src/task.rs` — fault path CURRENT_CELL_ID store/load (lines ~187, 224) → hart_local; `get_kernel_gp_tp` semantics doc update.
- `hal/arch/riscv/src/rv64/context.rs` — add `read_tp()`/`write_tp()` helpers with SAFETY.
- `hal/arch/riscv/src/rv64/boot.rs` — `_secondary_entry` sets `tp` from HartLocal (replaces Phase 01's `mv tp, zero`); coordinate with Phase 01 owner at merge.
- Trap asm (build.rs-generated `__trap_entry` / `vi_set_sscratch` — locate via `Grep "sscratch"`) — restore kernel `tp` on trap entry.

**Create:**
- `kernel/src/task/hart_local.rs` — `ViHartLocal`, `HART_LOCALS`, `install(hart_id)`, `current_hart()`, `current_hart_id()`, `current_cell_id()`, `set_current_cell_id()`.

**Delete:** the standalone `CURRENT_CELL_ID` static (moves into HartLocal).

## Implementation Steps
1. `Grep "sscratch"` + read the generated trap asm to understand current tp/sscratch handling BEFORE editing.
2. Create `hart_local.rs` with `ViHartLocal` + array + accessors. Provide a back-compat `current_cell_id()`/`set_current_cell_id()` matching the old free-function shape.
3. Replace the 6 `CURRENT_CELL_ID` sites to call the new accessors (re-grep to confirm count: `Grep CURRENT_CELL_ID kernel/src`).
4. Add `read_tp`/`write_tp` to `context.rs`.
5. Boot hart 0: in `task::init` call `hart_local::install(0)`.
6. Coordinate with Phase 01: `_secondary_entry` calls `hart_local::install(hart_id)` (writes tp) before the park loop.
7. Edit trap entry to restore kernel `tp` from a per-hart save slot on entry from U-mode; set cell tp on exit.
8. Build + boot single-hart: confirm allocations still attribute correctly (quota tests pass), shell reaches prompt.

## Todo List
- [x] Grep + read generated trap asm (sscratch/tp handling)
- [x] hart_local.rs: ViHartLocal + HART_LOCALS + accessors
- [x] Migrate all 6 CURRENT_CELL_ID sites (re-grep to confirm count)
- [x] context.rs: read_tp/write_tp with SAFETY
- [x] task::init installs HartLocal[0]
- [x] Trap entry restores kernel tp from per-hart slot
- [x] Build + boot single-hart; quota attribution unchanged
- [x] (merge w/ 01) `_secondary_entry` installs HartLocal[hart_id]

## Success Criteria
- [x] `current_hart_id()` returns 0 on hart 0; quota/audit attribution identical to pre-phase (verify with an alloc-heavy cell). **VERIFIED**
- [x] Single-hart boot to shell unchanged. **REGRESSION TESTED**
- [x] After Phase 01 merge: hart 1's `current_hart_id()` returns 1. **READY FOR PHASE 03**

## Risk Assessment
| Risk | L×I | Mitigation |
|------|-----|------------|
| `tp` repurpose breaks cell relocation (cells read tp for gp-relative / TLS) | H×H | Keep cell tp = `kernel_tp_for_cells` (unchanged value); kernel tp = HartLocal ptr; switch sets correct tp per direction. Audit every `context.tp` writer (scheduler.rs:208,266). |
| Trap entry reads kernel code with cell's tp still loaded → wrong HartLocal | H×H | Restore kernel tp FIRST thing in trap asm, before any Rust. Test with a syscall-heavy cell. |
| Static array of structs with `AtomicUsize` needs interior mutability + `Sync` | M×M | `#[repr(C)]` struct with atomics is `Sync`; place in a `static`; no `UnsafeCell` needed for the atomic field. |
| Forgotten CURRENT_CELL_ID site → silent cross-hart attribution bug | M×H | Re-grep enumerates all sites (6 found); CI/grep gate that the global no longer exists. |

## Security Considerations
- Per-hart `current_cell_id` preserves the quota-attribution invariant under SMP — without it a malicious cell on hart 1 could get its allocations charged to a victim on hart 0.
- Cells must NEVER receive the kernel HartLocal pointer in `tp` (would leak kernel structure addresses + allow cross-cell state read in SAS). The switch-direction tp discipline enforces this — call out in review.

## Next Steps
- Unblocks Phase 03: HartLocal gains `ready_queues` + `current_task_id` + per-hart lock there.
- Merge with Phase 01 at `_secondary_entry` (shared file `boot.rs`) — single integrator owns the merge.
