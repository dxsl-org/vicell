# Cellos Reliability Model — The "Never-Die" Spec

**Version**: 0.1 (Initial — Reliability Track Definition)
**Status**: Definitive
**Last Updated**: 2026-06-05

> Cellos targets robots and embedded devices. For that domain "fast + realtime" is not
> enough — the system **must not die**. This spec defines what "không chết" means
> concretely, records where Cellos stands today, and lists the criteria that must be
> completed. It is the canonical reference for the Reliability track.

---

## 1. What "Never-Die" Means — Six Independent Axes

"Never-die" is **not one property**. It decomposes into six axes that Cellos scores very
differently on. Conflating axis 1 (isolation) with the whole is the single biggest mistake.

| # | Axis | Core question | Example of "death" |
|---|------|---------------|--------------------|
| 1 | **Fault isolation** | Does one component's failure take down the system? | A driver crash panics the kernel |
| 2 | **Fault detection** | Can we detect a hung/dead component? | A cell spins in `loop{}`, nobody notices |
| 3 | **Fault recovery** | Can we auto-restart / self-heal? | A driver dies and stays dead |
| 4 | **Realtime guarantee** | Do we "die by deadline"? | Motor-control loop misses its deadline |
| 5 | **Continuous operation** | Update without downtime? | Must reboot to patch a bug |
| 6 | **HW fault tolerance** | Survive hardware faults? | RAM bit-flip, hung CPU |

For a robot, **axes 2–3 (detection + recovery) are what keep it from driving into a wall.**
A statically-isolated but unrecoverable system still "dies" the moment a control cell crashes.

---

## 2. Isolation Strategy Decision (2026-06-05)

**Per-Cell SATP isolation at Tier 1 is NOT pursued.** Rationale:

- Cellos runs in **RISC-V S-mode** under SBI. **PMP is M-mode-only** (Priv Spec §3.7) — an
  S-mode kernel cannot program it without custom M-mode firmware. **sPMP** (S-mode PMP) is
  **not ratified and not in commodity silicon** as of 2026. So PMP is viable only as a
  *static boot-time* guard, never as a per-cell dynamic isolator.
- Per-cell **SATP** is the only implementable hardware route, but it **breaks Tier 1
  zero-copy IPC** (different page tables can't share pointers — needs seL4-style shared-frame
  grants) and forces `sfence.vma` on every switch (**ASID is broken/absent on most RV
  silicon**, forcing full TLB flushes). The cost falls on the crown-jewel fast path.

**Resolution — isolation comes from the tiered model** ([05-application.md](05-application.md)),
not from retrofitting MMU into the SAS:

| Tier | Who runs here | Isolation mechanism |
|------|---------------|---------------------|
| **Tier 1 — Native (SAS)** | Signed, first-party, `#![forbid(unsafe_code)]` cells: drivers, FS, robot control | Language-Based Isolation (compiler) + signed-cells |
| **Tier 2 — Managed** | Third-party / portable code | WASM software sandbox (`wasmi` interpreter — no JIT escape) |
| **Tier 3 — Virtual** | Untrusted / legacy / sensitive silos | Hypervisor cell, **Stage-2 paging** (real hardware MMU barrier, *per-VM*) |

Hardware isolation thus lives in **Tier 3 (per-VM Stage-2)** — the right place for it —
**not** smeared across every Tier-1 cell. This **strengthens** the never-die story: with
Tier 1 restricted to signed safe-Rust, the only failure mode is a panic (caught + killed),
not silent memory corruption. Every Tier-1 death becomes a *restartable* event — which is
exactly what the supervisor track (below) handles.

> **Dependency this shifts onto Security:** the "Tier 1 = signed only" premise requires
> **code-signing + secure-boot + a loader gate** that refuses unsigned native ELF and routes
> untrusted code to Tier 2/3. Today "trusted" = *path is under `/bin/`* (a directory, not a
> crypto boundary). Ed25519 signing is spec-only. This is tracked separately as the Security
> track; it does not block the Reliability track but is load-bearing for the trust model.

---

## 3. Current Status — Scored per Axis

Grounded in the codebase as of 2026-06-05. Scores are relative to a production-grade
embedded/robotics OS (QNX/seL4 class), not relative to zero.

| Axis | Score | What exists | What's missing |
|------|------:|-------------|----------------|
| 1. Fault isolation | **~85%** | `panic_handler` isolates cell panics ([kernel/src/main.rs](../../kernel/src/main.rs)); trap handler kills faulting cell not kernel ([hal/arch/riscv/src/rv64/trap.rs](../../hal/arch/riscv/src/rv64/trap.rs)); per-cell heap quota ([kernel/src/memory/cell_quota.rs](../../kernel/src/memory/cell_quota.rs)); stack **guard pages active** ([stack.rs](../../kernel/src/task/stack.rs)); load-time VA-overwrite guard + build-time VA-layout CI check; async-pin/grant leak closed as moot (§4.4) | Depends entirely on zero-unsafe-bug in kernel/HAL; no per-cell SATP (by decision) |
| 2. Fault detection | **~78%** | Audit ring (`CellFault`/`CellExit`); CPU-monopoly watchdog (RT-only, reset-on-syscall); `RecvTimeout` deadline sweep checked in `pick_next`; **liveness heartbeat** (`Heartbeat=207` → `CellHung` kill→restart, catches silent hangs any priority); RT `RtDeadlineMiss`/`RtCpuOverrun` audit events ([kernel/src/audit.rs](../../kernel/src/audit.rs)) | No external HW watchdog; heartbeat is opt-in (only net adopts it so far) |
| 3. Fault recovery | **~88%** | Full multi-child supervisor via `NotifyOnExit` (init auto-restarts vfs/net/shell/…); per-service restart **policies** (permanent/transient/temporary) + **time-windowed restart intensity** (crash-storm escalation); exit-reason delivered as recv payload; service-ID registry (clients reconnect across respawn); hotswap + state-stash | App-level liveness heartbeat; cross-node failover (out of scope for single device) |
| 4. Realtime guarantee | **~45%** | 3-level priority preempt + zero-latency SSIP; RT watchdog; deadline-miss + CPU-overrun **observability** ([kernel/src/task/scheduler.rs](../../kernel/src/task/scheduler.rs)) | EDF / deadline enforcement / CPU-budget — **hardware-data-gated** (QEMU TCG has no cycle-accurate timing); WCET unmeasured |
| 5. Continuous operation | **~50%** | 5-step hotswap protocol ([kernel/src/cell/hotswap.rs](../../kernel/src/cell/hotswap.rs)); snapshot warm-boot | Partial rollback, message-queue preservation incomplete, manual trigger |
| 6. HW fault tolerance | **~5%** | — | No HW watchdog, no ECC, no redundancy/failover |

**Aggregate "never-die": ~25–30%.** Strong *prevention* foundation (tiny ~11.5K-LOC TCB,
Rust safety, working cell isolation). The *detection + recovery* layer — the part that
defines never-die for robots — is largely absent.

> **Spec/code mismatch to fix:** [01-core.md](01-core.md) §5 describes `catch_unwind`-wrapped
> inter-cell calls, automatic driver hardware-reset, and hot re-linking on panic. **None of
> that is implemented.** Actual behavior = `panic_handler` → `terminate_current_cell_on_fault`
> → cell killed, **no restart**. The supervisor work below makes §5's intent real; until then
> §5 is aspirational, not descriptive.

---

## 4. Completion Criteria — The Reliability Track

Ordered by ROI for never-die. Items are independent of the (dropped) SATP decision.

### 4.1 — Stop silent death (P0, cheap)
- [x] **Reboot-on-kernel-panic** — DONE 2026-06-06 (commit f7515e05). Kernel panic now requests an
      SBI SRST **cold reboot** (`sbi::system_reset`) after printing diagnostics, falling back to the
      halt loop only if firmware lacks SRST. Cell faults unaffected. Verified in QEMU (injected panic
      reboots vs freezes; normal boot still reaches `Cellos >`).
- [x] **Stack guard pages** — DONE 2026-06-06 (commit a8fa971c). Root cause of the earlier block
      found + fixed: the spawn paths zeroed the stack from `kstack.base` (the guard frame itself)
      for `STACK_FRAMES` pages — writing *through* the guard. Now zero from `base+PAGE_SIZE` (the
      usable pages only), then unmap the guard frame + `sfence.vma` in `stack.rs::allocate`. A stack
      overflow now traps. Verified: boot reaches `Cellos >` with guards active, 0 unmap failures.
      Remaining verification: a deliberate-overflow test cell to confirm the trap fires (follow-up).

### 4.2 — Detection (P0)
- [x] **Liveness heartbeat (silent-hang detection)** — DONE 2026-06-06 (commit b5c47c62). The
      watchdog only catches RT compute hogs; a cell that deadlocks or wedges in a stuck loop at any
      priority is "alive but paralyzed" and invisible to it. A cell opts in via `Heartbeat = 207`
      (Law 1, open syscall, `a0 = interval_ticks`, 0 = disable), asserting it will beat again within
      the interval; `pick_next` arms `Task.heartbeat_deadline` and terminates any cell that lapses
      (`CellHung` audit) → the death flows through `exit_task` so the supervisor restarts it. The net
      service is the reference adopter (beats once per poll iteration). **Live-verified both ways**: a
      healthy beating net survives boot (0 faults); an injected hang → "missed liveness deadline —
      terminating (hung)" → supervisor restart, no collateral, 0 panics.
- [x] **Kernel watchdog** — DONE 2026-06-06 (commit 0c34ff8f). `pick_next` charges a `run_ticks`
      per 10ms tick a task is found Running, reset on voluntary block AND on every syscall entry
      (cells are poll-based, so a syscall = progress). Crossing the 5s budget terminates the cell
      via `exit_task` + audit. **Scoped to RealTime priority only**: under preemptive round-robin,
      Normal/Background compute-heavy cells don't starve others, so killing them would false-positive
      (verified: a naive version killed bench/shell; RT-only fires 0× on a normal boot+bench). The
      RT-runaway kill path is logically exercised every tick; a dedicated RT-spin test cell is the
      remaining verification.
- [x] **Deadline enforcement** — DONE 2026-06-06 (commit f2623057). `pick_next` sweeps
      `Recv{deadline}` alongside `Sleeping{until}`; a timed-out receiver is woken with the timeout
      sentinel (`regs[10]=0`, matching ostd `sys_recv_timeout`'s `Ok(0)`). Closes
      infinite-block-on-dead-peer. Also reconciled the unit (10ms scheduler ticks; ostd doc was
      stale at 100ns — no cell calls it yet, so defined cleanly). Verified: no boot regression;
      positive timeout-fires path is unexercised until a cell uses RecvTimeout (follow-up test).

### 4.3 — Recovery: Supervisor Tree (P0, highest ROI)
Erlang/OTP-style "let it crash + restart".
- [x] **Supervisor MVP — init auto-restarts the shell** — DONE 2026-06-06 (commit 8113503c).
      init captures the shell tid and `sys_wait`s on it; on shell exit OR fault the kernel wakes
      the waiter (Phase 00 made fault paths notify waiters), and init respawns the shell, with a
      restart cap against crash-storms. Uses only `sys_wait` + `sys_spawn_from_path` — **no new
      ABI / no Law 1**. Functionally verified end-to-end: `exit` kills the shell, init logs
      "shell died — restarting" → "shell restarted", a 2nd `Cellos >` appears, init doesn't fault.
      > Prereq bug fixed first: init had a pre-existing instruction-fault during boot — the bench
      > cell lacked a linker script and clobbered init's `.text` PTE (commit e6798320). Also the
      > boot gate's fault pattern was broken and hid it (fixed). Both were masking init's death.
- [x] **Full multi-child supervision** — DONE 2026-06-06 (commits ca06abab + e1cf1abb).
      `ViSyscall::NotifyOnExit = 204` (Law 1, 2× confirmed) gives wait-any: `exit_task` delivers a
      death notification to each watcher (wakes a parked `Recv` returning the dead tid, or queues to
      `Task::pending_deaths` for the next `Recv` — never missed during respawn); SpawnCap-gated.
      init now supervises ALL services (vfs/config/input/net/compositor/shell) with one `sys_recv`
      loop, restarting whichever dies + re-arming. Verified: boot reaches `Cellos >` "supervising
      services"; exiting the shell → "service died — restarting"/"service restarted", 2nd prompt, 0
      panics.
- [x] **Stable service-ID registry** — DONE 2026-06-06 (commit 5cda48d8). Kernel `service_id→tid`
      map so a restarted vfs/net keeps its endpoint for clients (`RegisterService`/`LookupService`,
      Law 1; supervisor-owned namespace; `clear_tid` on death). See §4 Axis 1/3. → [[service registry]]
- [x] **Restart policies + intensity** — DONE 2026-06-06 (commit 40ad2996). Per-service Policy
      {Permanent, Transient (restart only on abnormal exit), Temporary (never)} + per-service
      time-windowed restart **intensity** (≤5 / ~10 s via `sys_get_time`; a crash storm escalates —
      give up on that one service — instead of spin-respawning or burning a shared global budget).
      Needed the **exit reason** at the supervisor: the kernel now delivers it as the `Recv` payload
      (the NotifyOnExit contract), stashed in `exit_task` and written to the watcher's buffer when
      its `Recv` RESUMES (the watcher's own syscall context — writing it from the trap/fault context
      faults: S-mode store to a USER page with SSTATUS.SUM unset; that bug was caught + fixed in
      test). Live-verified: `exit` → shell faults (reason=MAX) → died/restarting/restarted, new tid +
      prompt, exactly 1 fault, 0 panics.
- [ ] Remaining polish (not blocking): `parent_cell_id` for finer watch-gating; app-level liveness
      heartbeat. **Shell `exit` fault FIXED** (commit 844409f4): its root cause was the cell
      heap leak below — the shell OOM'd during command processing and store-faulted. With the
      freeing allocator + a direct `sys_exit`, `exit` now exits cleanly (reason 0) and init's
      Transient policy keeps it down; a crash still restarts.

### 4.4 — Stop slow death (P1)
- [x] **Freeing cell heap allocator (userspace)** — DONE 2026-06-06 (commit 844409f4). The biggest
      slow-death source: `ostd`'s allocator was a bump allocator whose `dealloc` was a NO-OP, so
      EVERY cell leaked all allocations and eventually exhausted its 4 MiB arena → null alloc →
      store-fault. A guaranteed death for any long-running cell (shell, all services). Replaced with
      `linked_list_allocator` (kernel-shared crate) via a `static mut Heap` — no spinlock, because a
      `LockedHeap`'s atomic write-back faults when the const-init allocator static lands in a cell's
      read-only RELRO segment. OOM now exits the cell for supervised restart (fresh heap) instead of
      hanging. Companion linker-script fix (all 10 cell `.ld`): place `.data.rel.ro`/`.got` in
      writable `.data`, and page-align trailing read-only sections off `.bss`'s last page (the loader
      maps that shared page RW for `.bss` then remaps it read-only for the manifest/`.eh_frame`,
      faulting writes to `.bss` globals such as the heap state). Verified: 0 boot faults; cells can
      now run indefinitely.
- [x] **Reap zombies → free dead-cell stacks** — DONE 2026-06-06 (commit 6bb1cc3a). Zombies were
      never removed, so `Stack::drop` never ran and every cell death leaked its kernel+user stacks.
      `Scheduler::take_reapable_zombies` (called from `yield_cpu`, dropped outside the SCHEDULER lock
      for lock-order safety) now frees them. Verified: 3 forced shell crash→reap→restart cycles,
      0 kernel panics, no reaper UAF/deadlock.
- [x] **Free ELF segment frames on cell death** — DONE 2026-06-06 (commit 82fc085a). `load_segments`
      returns the mapped `(vaddr, frame)` pairs; the Task owns them as `CellSegments`, freed when the
      zombie is reaped (outside the SCHEDULER lock). Race-safe with same-VA respawn: `CellSegments::drop`
      only unmaps a VA that still resolves to its own frame (else respawn already re-pointed it).
      Verified: 3 crash→reclaim→restart cycles, all restarts reach the prompt, 0 panics.
- [x] **`load_segments` overwrite-guard** — DONE 2026-06-06 (commits 6f5dd2b9 + 9ce3cb6b). The SAS
      silent-corruption defense: a cell loading at an already-mapped VA is rejected (collision with a
      live cell OR kernel MMIO) instead of silently clobbering the PTE. The guard's first run was NOT
      a false-positive — it **found a real latent bug**: vfs (`0x2000000`) sat inside CLINT and
      bench/lua (`0xC000000`) + micropython (`0xE000000`) sat inside the PLIC MMIO identity map
      (paging.rs:140-148), so loading them clobbered interrupt-controller MMIO PTEs. Fixed by
      relocating those four cells above all MMIO (≥0x1001_0000, <RAM), mutually disjoint. Guard
      details: skips a cell's own intra-ELF overlaps (the load's `mapped` set); rolls back partials
      on reject; `CellSegments::eager_unmap` frees a dying cell's VAs at death so respawn (fixed VA)
      isn't blocked. Verified: 0 false-fires on boot, shell crash→respawn works, 0 panics.
- [x] **GC for async-pinned buffers / grants — CLOSED as MOOT 2026-06-06** (scoping report:
      `.agents/reports/scope-260606-1454-p05-async-pin-gc.md`). Verified leak-free **by
      construction**, three independent reasons: (1) the async FileRead future is Task-owned
      and pins no separate frame — it captures a raw pointer into the cell's OWN buffer
      (task.rs:567-592); a cell killed while `Polling` is removed from `self.tasks` in
      `exit_task` BEFORE its frames free, and the poll loop only polls `self.tasks`, so the
      dangling write never executes. (2) The inner read is synchronous (fat.rs:415-425) — no
      DMA descriptor outlives the future. (3) Grant/lease IPC cannot be created at runtime —
      `ostd::sys_grant` is a stub (ostd/syscall.rs:538), so `grant_table`/`leases` are always
      empty (and are Task-owned metadata, freed at reap, holding no frames). **Future-work
      trigger:** a real async-DMA driver (the fat.rs TODO) or SMP makes this real — the
      cancellation point is `exit_task` (descriptor-cancel / frame-unpin before reclaim),
      documented inline there. With this closed, **P05 (stop slow death) is complete**: zombie
      reaper + stack reclaim + segment reclaim + overwrite-guard + async/grant verified safe.

### 4.5 — Realtime hardening (P1–P2)
- [x] **RT observability (P06 slice, DONE 2026-06-06).** `RtDeadlineMiss` audit event + per-task
      `deadline_misses` counter (emitted when an RT cell's `RecvTimeout` deadline elapses — a missed
      control-loop cycle); `RtCpuOverrun` one-shot audit at 80% of the watchdog budget (early warning
      before the hard kill). Built on existing primitives, no new ABI, no scheduler-policy change —
      makes RT failures *visible* so enforcement can be tuned once real-hardware bench data exists.
- [ ] CPU budget / time-slice guarantees per priority; measure WCET of syscall + IPC paths.
      **Hardware-data-gated:** QEMU TCG has no cycle-accurate timing, so WCET/EDF enforcement cannot
      be meaningfully validated here — defer to real-board bring-up (the RT bench scenarios exist).
- [ ] Evaluate EDF or deadline-aware scheduling for hard-RT control cells (after WCET data).

### Target trajectory
Completing 4.1–4.3 lifts **Detection ~15%→~65%** and **Recovery ~10%→~70%**, raising aggregate
never-die to **~55–60%** — the threshold where "OS for robots" becomes a fair description.

---

## 5. Prior Art — State of the Field

**No single OS achieves all six axes.** The axes pull in opposite directions, so real systems
specialize. Scoring the strongest contenders (✅ strong · 🟡 partial/conditional · ❌ weak/delegated):

| OS / Runtime | 1 Isolation | 2 Detection | 3 Recovery | 4 Realtime | 5 Hot-update | 6 HW fault-tol |
|---|---|---|---|---|---|---|
| **QNX Neutrino** | ✅ MMU | ✅ HAM watchdog | ✅ restart | ✅ hard RT | 🟡 per-component | ❌ needs redundant HW |
| **INTEGRITY** (Green Hills) | ✅ separation kernel | ✅ | ✅ | ✅ hard RT | 🟡 | ❌ |
| **seL4** | ✅ *proven* | ❌ DIY | ❌ DIY | ✅ *proven WCET* | ❌ | ❌ |
| **Erlang/OTP** (BEAM) | 🟡 in-VM only | ✅ | ✅ supervision tree | ❌ soft RT (GC) | ✅ hot code load | 🟡 via distribution |
| **HP NonStop** (Tandem) | ✅ | ✅ | ✅ process-pairs | ❌ not RT | ✅ online upgrade | ✅ lockstep HW |
| **VxWorks** | 🟡 | ✅ watchdog | 🟡 | ✅ hard RT | 🟡 remote patch | 🟡 redundant configs |

### Why no OS gets all six
- **Axis 6 is a *system/hardware* property, not an OS property.** Surviving a dead CPU or a RAM
  bit-flip requires *physical redundancy* (lockstep, TMR, ECC, replicas). An OS on a single chip
  cannot provide it regardless of code quality — the *co-designed system* (HP NonStop, Stratus
  ftServer) does. Claiming "an OS achieves axis 6" is nearly a category error.
- **Axis 4 (hard RT) ↔ Axis 5/6 tension.** Deterministic deadlines fight jitter-introducing
  mechanisms (live update, failover, consensus). Erlang takes 5, sacrifices 4; QNX takes 4, is
  cautious on 5.
- **"All six" exists only in co-designed safety-critical *systems*** — fly-by-wire (dissimilar
  redundancy voting across multiple CPUs+RTOSes), FADEC, nuclear/medical (TMR + HW watchdog +
  certified RTOS). That is `certified RTOS (axes 1–5) × redundant hardware (axis 6)`, not one OS.
- Even the best is *asymptotic*: "nine nines" (≈Ericsson AXD301/Erlang), not literal infinity.

### The two never-die regimes — and why "scalable systems look closest to 6"
The systems that *scale out* (NonStop, Erlang, and by extension Spanner/Borg/Kubernetes) appear
to "almost have all six" because **horizontal scale = redundancy = the mechanism for axis 6
(and it boosts 2/3/5) without special fault-tolerant silicon.** If one node dies, peers take
over; replication buys hardware fault tolerance the cheap way. NonStop is the proof point: it
scales to thousands of CPUs *and* gets axis 6 via lockstep — missing only hard-RT (axis 4).

But the catch is structural: **the very mechanism that buys axis 6 by scaling (replication,
failover, consensus across nodes) injects non-determinism that kills axis 4.** So "scalable ⇒
6 axes" is really "scalable ⇒ availability (1,2,3,5,6) *minus* hard realtime". There are thus
two regimes, on opposite ends:

| | **Availability regime** (scale-out) | **Safety/RT regime** (embedded) |
|---|---|---|
| Examples | NonStop, Erlang, K8s, Spanner | QNX, INTEGRITY, VxWorks, seL4 |
| "Never-die" means | the *service* survives though nodes die constantly | this *one device* keeps meeting deadlines & fails safe |
| Axis 6 via | distribution + replication (cheap, no special HW) | on-board redundancy (TMR/lockstep) or safe-state |
| Sacrifices | hard realtime (axis 4) | cheap axis 6 (a single robot body can't scale out) |

**Key limit for robots:** you cannot horizontally scale a robot's *physical body* — actuators
are singular. So for a single robot, axis 6 must come from on-board redundancy or graceful
safe-state, not scale-out. Scale-out's free axis 6 applies to Cellos's *cloud-microservice*
use case, not its motor-control use case.

### The unifying insight (Cellos-relevant)
**Supervisor-restart (one node) and node-failover (distributed) are the same recovery pattern at
different scales** — "let it crash, something restarts it". Cellos's cell + supervisor-tree model
(Phases 03–04) is the single-node form. Because cells communicate only via IPC (location-agnostic
by design), the *same* supervision/abstraction can later extend across nodes (distributed cells):

- For **cloud microservices** (a Tier-1 use case in [05-application.md](05-application.md)):
  cross-node cell failover is Cellos's path to axis 6 in the availability regime — for free,
  as a byproduct of scaling the existing model. **Do not build this now (YAGNI)**, but the
  supervisor/IPC ABI should not foreclose it.
- For **robot fleets/swarms:** one robot dying while the swarm continues is fleet-level axis 6,
  again the same supervision pattern lifted one level.

**Conclusion for Cellos:** the realistic single-OS target is **QNX-class on axes 1–5** (trusted-tier
model), with **axis 6 pushed to deployment hardware** (ECC, HW watchdog, redundant nodes) — and
the cell+supervisor model kept *scale-ready* so the availability-regime path to axis 6 stays open
later. Cellos's differentiator vs QNX (C) is **Rust LBI + ~11.5K-LOC TCB**: no existing OS
combines Rust safety + a tiny TCB + Erlang-style supervision. That intersection is the niche.

---

## 6. Cross-References

- Tiered isolation model: [05-application.md](05-application.md)
- Panic/fault behavior + capabilities: [01-core.md](01-core.md) §5 (note mismatch above)
- Scheduler & realtime preemption: [03-runtime.md](03-runtime.md)
- Security track (signing, secure-boot, Spectre): [../security-model.md](../security-model.md)
- Deadlock watchdog (test harness): [10-testing.md](10-testing.md), [04-hardware.md](04-hardware.md)
