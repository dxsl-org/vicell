# Phase 03 — SSIP Self-IPI for Zero-Latency RT Wakeup

**Status**: 📋 PLANNED  
**Priority**: P1  
**Effort**: 2 days  
**Depends on**: Phase 01 + Phase 02

---

## Context Links

- Trap handler: `hal/arch/riscv/src/rv64/trap.rs`
- Syscall dispatch: `kernel/src/task/syscall.rs`
- IPC wakeup paths: `kernel/src/task/scheduler.rs` (Recv/Send state transitions)
- RISC-V SIP CSR: privileged spec §4.1.3 — SSIP is R/W from S-mode

---

## Overview

With Phase 01 (timer only), a newly-spawned RealTime cell waits up to 10 ms for the next timer tick before preempting a Normal cell. Phase 03 eliminates this latency by sending a software interrupt to self (`csrsi sip, 0x2`) immediately when a RealTime cell becomes runnable.

**When to pend SSIP (S-mode software interrupt):**
1. A RealTime cell is spawned (`spawn_with_priority(RealTime)`)
2. A RealTime cell is unblocked (IPC reply, sleep expiry, file read completion)

In both cases: if the priority of the newly-ready task > current task's priority, pend SSIP. The interrupt fires at the next point where `sstatus.SIE=1`, which is effectively immediate (within the current syscall handler's exit sequence).

---

## Requirements

- SSIP fires within the same kernel entry as the wakeup syscall — no additional round-trips
- SSIP handler calls `pick_next()` and performs context switch to the RealTime cell
- Timer interrupt (Phase 01) remains the primary preemption mechanism for timeslice expiry
- SSIP interrupt clears `sip.SSIP` before calling `pick_next()` (prevents re-entry)
- No priority inversion: Normal cells holding a spinlock are NOT interrupted mid-lock (interrupts already disabled during spinlock hold by Spinlock<T>)

---

## Key Insights

### Why SSIP and not MSIP
ViCell runs in S-mode under OpenSBI. `msip` (CLINT offset 0x0) is M-mode and blocked by OpenSBI. `sip.SSIP` (bit 1) is writable from S-mode per the RISC-V privileged spec — this is the correct mechanism.

### CSR sequence
```rust
// Pend software interrupt to self:
// SAFETY: csrsi on sip.SSIP is permitted from S-mode (RISC-V priv spec §4.1.3)
unsafe { core::arch::asm!("csrsi sip, 0x2") }

// Inside SSIP handler (scause = 0x8000_0000_0000_0001, code=1):
// SAFETY: csrci on sip.SSIP clears the pending bit; must happen before re-enabling SIE
unsafe { core::arch::asm!("csrci sip, 0x2") }
```

### `sie.SSIE` must be enabled
Add `csrsi sie, 0x2` alongside `csrsi sie, 0x20` (STIE) in HAL init. Otherwise SSIP pends but the interrupt never fires.

### Interrupt nesting consideration
The current trap handler does NOT re-enable `sstatus.SIE` inside the handler body, so nested interrupts are impossible. This is safe: SSIP fires when the current ISR returns (via `sret`) and `sstatus.SIE` is restored. For the preemption use case (wakeup inside a syscall), the syscall exits via `sret`, restoring SIE, and SSIP fires immediately — which is the correct behavior.

---

## Related Code Files

### Modify
- `hal/arch/riscv/src/rv64.rs` — add `csrsi sie, 0x2` (SSIE enable) in `Arch::init()`
- `hal/arch/riscv/src/rv64/trap.rs` — handle `scause == 1` (S-mode software interrupt): clear SSIP + call `yield_from_timer(frame)`
- `kernel/src/task/scheduler.rs` — add `pend_preempt_if_needed(new_priority: u8)` helper
- `kernel/src/task/syscall.rs` — call `pend_preempt_if_needed` after any state transition that could unblock a higher-priority task

---

## Implementation Steps

### Step 1 — Enable SSIE in HAL init

In `hal/arch/riscv/src/rv64.rs`, alongside the STIE enable from Phase 01:
```rust
// Enable S-mode software interrupt (SSIP preemption path for RT wakeup).
// SAFETY: csrsi on sie is safe from S-mode; sets SSIE (bit 1) only.
unsafe { core::arch::asm!("csrsi sie, 0x2"); }
```

### Step 2 — Handle scause==1 in trap.rs

Currently `code == 1` falls to the "unknown interrupt" branch. Add:

```rust
1 => {
    // S-mode software interrupt — RT preemption triggered by SSIP pend.
    // Clear SSIP first to prevent re-entry before re-enabling SIE.
    // SAFETY: csrci on sip.SSIP is permitted from S-mode.
    unsafe { core::arch::asm!("csrci sip, 0x2") };
    crate::task::yield_from_timer(frame); // reuse existing pick_next + switch
}
```

No timer rearm here — only the timer ISR (code==5) rearmed the timer.

### Step 3 — Add `pend_preempt_if_needed` to scheduler

```rust
/// Pend an S-mode software interrupt if `new_priority` exceeds the current
/// running task's priority.  The interrupt fires when the current kernel
/// entry returns via `sret` and `sstatus.SIE` is restored.
///
/// Call this after any syscall that transitions a task from blocked → Ready.
pub fn pend_preempt_if_needed(&self, new_priority: u8) {
    let current_priority = self.current_task_id
        .and_then(|id| self.tasks.get(&id))
        .map(|t| t.priority)
        .unwrap_or(0);

    if new_priority > current_priority {
        // SAFETY: csrsi on sip.SSIP is permitted from S-mode (priv spec §4.1.3).
        // The interrupt fires after sret restores sstatus.SIE.
        unsafe { core::arch::asm!("csrsi sip, 0x2") };
    }
}
```

### Step 4 — Call pend_preempt_if_needed at wakeup sites

In `kernel/src/task/syscall.rs`, after every path that sets a task state to `Ready`:
```rust
// Example: IPC reply unblocks a waiting task
task.state = TaskState::Ready;
sched.ready_queues.entry(task.priority).or_default().push_back(task_id);
sched.pend_preempt_if_needed(task.priority);
```

Key wakeup sites to cover:
- IPC reply (SYS_REPLY / SYS_CALL_REPLY)
- Sleep expiry (`Sleeping → Ready`, already done in pick_next — SSIP not needed here since timer fires anyway)
- File read completion (`Polling → Ready`)
- Spawn of a new cell (if RT, pend immediately)

---

## Todo List

- [ ] Add `csrsi sie, 0x2` (SSIE) in HAL init
- [ ] Handle `scause == 1` in `trap.rs` (clear SSIP + yield_from_timer)
- [ ] Add `pend_preempt_if_needed(priority)` to `Scheduler`
- [ ] Call `pend_preempt_if_needed` in IPC reply path
- [ ] Call `pend_preempt_if_needed` in spawn path for RealTime cells
- [ ] Call `pend_preempt_if_needed` in file-read completion path
- [ ] `cargo check -p vicell-kernel` — no errors
- [ ] Integration test: RT cell spawned while Normal cell runs → preempts within same syscall exit

---

## Success Criteria

- [ ] RT cell preempts Normal cell within < 1 µs of wakeup (measured via `read_mtime()` delta in test)
- [ ] Normal cell resumes correctly after RT cell yields or exits
- [ ] No double-preemption (SSIP fires once per wakeup event, not repeatedly)
- [ ] All 65 integration tests pass
