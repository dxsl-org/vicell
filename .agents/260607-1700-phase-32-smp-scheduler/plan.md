---
title: "Phase 32: SMP Multi-Core Scheduler"
description: "Per-hart run queues + work stealing + RT-pinned hart + IRQ-driven net waker on 2-hart QEMU virt"
status: complete
priority: P3
effort: ~4 weeks
branch: main
tags: [smp, scheduler, riscv, kernel, hal, g2]
created: 2026-06-07
---

# Phase 32 — SMP Multi-Core Scheduler

Take ViCell from single-hart to 2-hart (main work hart + dedicated RT hart) on QEMU virt.
Per-hart ready queues with work stealing, RT cells pinned to hart 1, and an IRQ-driven
waker so the net cell stops busy-polling. Roadmap: `docs/project-roadmap.md` §Phase 32 (line 721).

## Prime-directive grounding (verified)
- Scheduler is one global `Spinlock<Option<Scheduler>>` — `kernel/src/task.rs:32`. Serializes ALL ops.
- `CURRENT_CELL_ID: AtomicUsize` (single) — `kernel/src/task/scheduler.rs:12`.
- `pend_preempt_if_needed` writes SSIP locally only — `scheduler.rs:145`.
- Boot asm sets `tp=0`, single-hart, `a0`=hartid → kmain — `hal/arch/riscv/src/rv64/boot.rs`.
- `sbi.rs` has NO HSM (`hart_start`) and NO IPI — `hal/arch/riscv/src/common/sbi.rs`.
- Software-interrupt (scause code 1) already wired to `vi_timer_tick` — `rv64/trap.rs:67`.
- Net cell busy-polls on a tick interval — `cells/services/net/src/main.rs:96`.

## Phase Table

| # | Phase | Status | BlockedBy | Effort | Description |
|---|-------|--------|-----------|--------|-------------|
| 01 | [SBI HSM + per-hart boot](phase-01-sbi-hsm-hart-boot.md) | ✅ Done | — | ~1 wk | Add `hart_start`/`send_ipi` to sbi.rs; bring hart 1 up to an idle park loop. No scheduler change. |
| 02 | [Per-hart local state](phase-02-per-hart-local-state.md) | ✅ Done | — | ~1 wk | `ViHartLocal` struct via `tp` CSR; `CURRENT_CELL_ID` → per-hart array. No queue change. |
| 03 | [Per-hart queues + work stealing](phase-03-per-hart-queues-work-stealing.md) | ✅ Done | 01✅, 02✅ | ~1.5 wk | Split global lock into per-hart spinlocks; idle hart steals half of busiest Normal backlog. |
| 04 | [RT pinning + IPI preempt + waker](phase-04-rt-pinning-ipi-waker.md) | ✅ Done | 03✅ | ~1 wk | RT→hart 1 (no steal); cross-hart IPI preempt; IRQ-driven net waker. Smoke: RT + Normal concurrently. |

Phases 01 and 02 are independent (different files, no shared symbol mutation) and run in PARALLEL.
03 merges both. 04 builds on 03.

## Key Dependencies
- 01 unblocks 03 (needs second hart actually executing the scheduler loop).
- 02 unblocks 03 (needs `tp`-based hart identity to index per-hart queues).
- 03 unblocks 04 (RT pinning and cross-hart IPI presuppose per-hart queues exist).
- External: read hermit-os `src/scheduler/` and embassy `embassy-net/src/` before 03/04 (roadmap requirement).

## File-ownership map (no two parallel phases touch the same file)
- Phase 01: `hal/arch/riscv/src/common/sbi.rs`, `hal/arch/riscv/src/rv64/boot.rs`, new `kernel/src/task/smp.rs`.
- Phase 02: `hal/arch/riscv/src/rv64/context.rs` (tp helpers), new `kernel/src/task/hart_local.rs`, `scheduler.rs` (CURRENT_CELL_ID only).
- 01 and 02 share NO file. Merge point is Phase 03 in `scheduler.rs` + `task.rs`.

## Risk Summary (full detail per phase)
| Risk | L×I | Phase | Mitigation |
|------|-----|-------|------------|
| Lock-order inversion across harts (SCHEDULER→FRAME deadlock multiplied) | H×H | 03 | Keep "release sched lock before FRAME" rule; per-hart lock is leaf; steal acquires victim lock with strict hart-id ordering (lower id first) to prevent ABBA. |
| `tp` clobbered by existing code that assumes tp=kernel-tp | H×H | 02 | Audit every `get_kernel_gp_tp`/`context.tp` user; tp now points to HartLocal, kernel gp/tp for cells stored IN HartLocal. |
| WaitForEvent syscall = libs/api change = **Law 1 (2x confirm)** | M×H | 04 | Get explicit 2x user confirm BEFORE coding the syscall; fallback = kernel-internal event table keyed by existing Recv. |
| Boot race: hart 1 runs scheduler before hart 0 finishes init | H×H | 01 | hart 1 parks in WFI on a per-hart "go" flag; hart 0 sets flag only after SCHEDULER + queues initialized. |
| Async future / DMA buffer freed on hart A while hart B holds raw ptr | H×H | 03 | Honor the `exit_task` INVARIANT note (scheduler.rs:316-324): add frame-unpin point before reclaim under SMP. |
| QEMU TCG serializes harts → false "it works" timing | M×M | 04 | Validate ordering/correctness, not wall-clock speedup; assert both harts advance distinct cells via audit log.|

## Success Criteria (phase-level "done")
- 01: `dmesg` shows "hart 1 online, parked"; hart 1 reaches its idle loop; hart 0 boots to shell unchanged.
- 02: each hart reads its own id via `tp`; `CURRENT_CELL_ID[hart]` attributes allocations correctly; single-hart path still boots.
- 03: a Normal cell spawned on a busy hart 0 gets stolen and run by idle hart 1; no deadlock under stress (N cells churn).
- 04: RT timer cell runs ONLY on hart 1, Normal cell runs on hart 0, both make progress in the same window (audit trace); net cell wakes on NIC RX IRQ, not on a poll timer.

## Open Questions
1. Does the user approve the Law 1 `WaitForEvent` syscall (215), or prefer the kernel-internal-event fallback? (blocks 04)
2. MAX_HARTS = 2 fixed, or parameterized for future N-hart? (KISS → fix at 2 for G2 entry, leave const.)
3. QEMU `-smp 2` boot: does Limine path also need a per-hart entry, or is direct `-kernel` the only target for Phase 32?
