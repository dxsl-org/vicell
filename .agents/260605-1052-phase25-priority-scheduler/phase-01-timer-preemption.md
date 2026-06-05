# Phase 01 — Timer Preemption Foundation

**Status**: 📋 PLANNED  
**Priority**: P0 (blocks all other phases)  
**Effort**: 2 days

---

## Context Links

- Timer interrupt stub: `hal/arch/riscv/src/rv64/trap.rs:67-72`
- Timer read/set: `hal/arch/riscv/src/common/timer.rs`
- SBI timer: `hal/arch/riscv/src/common/sbi.rs:62-68`
- Tick counter: `kernel/src/task.rs:34-77`
- HAL init: `hal/arch/riscv/src/rv64.rs`
- Scheduler pick_next: `kernel/src/task/scheduler.rs:182-331`

---

## Overview

The timer interrupt handler at `trap.rs:67` is currently an **empty stub**. `sie.STIE` is never enabled, the initial timer is never armed, and `task::tick()` is never called. Tasks only yield cooperatively via `yield_cpu()`.

This phase wires the full timer preemption path:

```
mtime expires → trap fires (scause=5) → tick() → pick_next() → context_switch()
                                                              → rearm timer
```

---

## Requirements

- Timer fires every 10 ms (10,000 µs; at 10 MHz mtime clock = 100,000 ticks per interval)
- `system_ticks()` increments by 1 on each timer interrupt
- Sleeping tasks (`TaskState::Sleeping { until }`) wake up on the correct tick
- Context switch from timer interrupt uses the existing `__switch()` assembly
- Timer is rearmed immediately after context switch so the next interval fires on time
- All 65 existing integration tests continue to pass

---

## Key Insights

### Why STIE must be explicitly enabled
RISC-V spec: `sie.STIE` (bit 5) defaults to 0 after reset. Without setting it, the timer interrupt is masked even if `sstatus.SIE=1`. Fix: `csrsi sie, 0x20` in HAL init.

### Timer CSRs (S-mode)
```
mtime   — read via `csrr t0, time` (time CSR, 0xC01, S-mode mirror of machine mtime)
mtimecmp — NOT directly accessible from S-mode. Use SBI call to set next deadline:
           sbi::set_timer(mtime + TICKS_PER_SLICE)
```

### SBI timer rearm sequence
```rust
// After context switch, inside timer ISR:
let next = hal::timer::read_mtime() + TICKS_PER_10MS;
hal::sbi::set_timer(next);
// OpenSBI clears sip.STIP automatically when mtimecmp is updated.
```

### Context switch from interrupt
The timer ISR must:
1. Save current task context into TCB (via `__switch` or inline)
2. Call `pick_next()` to select next task
3. Load new task context
4. Return from interrupt (sret)

The existing `__switch(old, new)` at `switch.S` saves/restores kernel context. It is designed for cooperative calls from `yield_cpu()`. From a trap handler, the trap frame is already saved on the stack by the trap entry assembly. The timer ISR must call into the scheduler the same way `yield_cpu()` does.

---

## Related Code Files

### Modify
- `hal/arch/riscv/src/rv64.rs` — add `csrsi sie, 0x20` and initial `set_timer()` in `Arch::init()`
- `hal/arch/riscv/src/rv64/trap.rs:67-72` — implement timer ISR: `tick()` + `pick_next()` + rearm
- `kernel/src/task.rs` — export `TICKS_PER_10MS` constant; ensure `tick()` is pub

### No change needed
- `hal/arch/riscv/src/rv64/asm/switch.S` — already correct for context save/restore
- `kernel/src/task/scheduler.rs:pick_next()` — already returns `(old_ctx, new_ctx)` pointers

---

## Implementation Steps

### Step 1 — Enable STIE and arm initial timer in HAL init

In `hal/arch/riscv/src/rv64.rs`, inside the `impl Arch for RiscV64 { fn init() { ... } }` block, add after PLIC setup:

```rust
// Enable S-mode timer interrupt.
// SAFETY: csrsi on sie is safe from S-mode; sets STIE (bit 5) only.
unsafe { core::arch::asm!("csrsi sie, 0x20"); }

// Arm the first timer tick (10 ms from now).
let now = crate::common::timer::read_mtime();
crate::common::sbi::set_timer(now + crate::common::timer::TICKS_PER_10MS);
```

Add the constant to `timer.rs`:
```rust
/// Ticks in 10 ms at the assumed 10 MHz mtime clock on QEMU virt.
pub const TICKS_PER_10MS: u64 = 100_000;
```

### Step 2 — Implement timer ISR in trap.rs

Replace the empty `scause == 5` branch (trap.rs:67-72):

```rust
5 => {
    // S-mode timer interrupt — preemption point.
    // Increment the global tick counter so sleeping tasks can wake.
    crate::task::tick();

    // Rearm timer for the next 10 ms slice before context switch,
    // so the deadline is anchored to the actual mtime, not to our
    // execution time.
    let next = hal::common::timer::read_mtime()
        + hal::common::timer::TICKS_PER_10MS;
    hal::common::sbi::set_timer(next);

    // Run the scheduler: if a higher-priority task is ready (or the
    // current task's slice expired), context-switch to it.
    // pick_next_from_trap() must be callable from an interrupt context
    // and must NOT re-enable interrupts mid-switch.
    crate::task::yield_from_timer(frame);
}
```

Add `yield_from_timer(frame: &mut ViTrapFrame)` in `kernel/src/task.rs`:

```rust
/// Called from the S-mode timer ISR.
///
/// Runs the scheduler and performs a context switch if a different task
/// should run next.  Must be called with interrupts disabled (they are
/// disabled on trap entry by the RISC-V hardware).
pub fn yield_from_timer(frame: &mut ViTrapFrame) {
    let mut sched = SCHEDULER.lock();
    if let Some(sched) = sched.as_mut() {
        if let Some((old_ctx, new_ctx)) = sched.pick_next() {
            drop(sched); // release lock before context switch
            // SAFETY: old_ctx and new_ctx point into Task structs that live
            // for the lifetime of SCHEDULER.  The context switch saves sepc/
            // sstatus into the Context struct so sret returns to the new task.
            unsafe { hal::arch::context_switch(old_ctx, new_ctx) };
        }
    }
}
```

### Step 3 — Verify sleeping tasks still wake correctly

`pick_next()` already calls `tick()` check at lines 194-206. With timer interrupts now firing, the tick count advances autonomously, so sleeping tasks wake without needing a busy-poll loop.

Run existing integration tests to confirm no regressions.

---

## Todo List

- [ ] Add `TICKS_PER_10MS` constant to `hal/arch/riscv/src/common/timer.rs`
- [ ] Enable `sie.STIE` in HAL init (`hal/arch/riscv/src/rv64.rs`)
- [ ] Arm initial timer in HAL init
- [ ] Implement `yield_from_timer(frame)` in `kernel/src/task.rs`
- [ ] Replace timer ISR stub in `trap.rs:67-72`
- [ ] Run `cargo check -p vicell-kernel` — confirm no compile errors
- [ ] Run integration tests — confirm all 65 pass

---

## Success Criteria

- [ ] QEMU boot log shows `[boot] kernel_phys_base=...` and later the shell prompt (no hang)
- [ ] `system_ticks()` increments over time (visible via a diagnostic log in shell)
- [ ] Sleeping `sys_sleep(n)` wakes within ≤2 timer intervals of the requested duration
- [ ] All 65 integration tests pass

---

## Risk Assessment

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| Context switch from ISR corrupts stack | Medium | Timer ISR must NOT use `__switch` directly; use a dedicated `yield_from_timer` that calls scheduler then context_switch |
| Timer fires before scheduler is initialized | Low | Arm timer after `task::init()` in `main.rs`, not in HAL init |
| Double-lock on SCHEDULER spinlock | Medium | `yield_from_timer` must drop the lock before calling `context_switch` |
| Sleeping task wakes one tick early (off-by-one) | Low | The existing `until <= current_tick` check is correct; no change needed |
