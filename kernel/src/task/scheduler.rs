use super::tcb::{FileHandle, SyscallFuture, Task, TaskState};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;
use core::task::{Context, Poll};
use log::info;
use types::*;

/// Read the currently-executing cell ID (0 = kernel).
///
/// Delegates to `hart_local` which reads the per-hart `current_cell_id` field
/// via the `tp` CSR — O(1), no lock.  Safe to call from the allocator hot path.
pub fn current_cell_id() -> usize {
    super::hart_local::current_cell_id()
}

// Dummy Waker
// In a real executor, we'd have a way to wake specific tasks.
// Here we just poll in the loop.
// We need a dummy waker to pass to poll.
use core::task::{RawWaker, RawWakerVTable, Waker};

fn dummy_waker() -> Waker {
    unsafe { Waker::from_raw(dummy_raw_waker()) }
}

fn dummy_raw_waker() -> RawWaker {
    RawWaker::new(core::ptr::null(), &DUMMY_VTABLE)
}

static DUMMY_VTABLE: RawWakerVTable =
    RawWakerVTable::new(|_| dummy_raw_waker(), |_| {}, |_| {}, |_| {});

/// CPU-monopoly watchdog budget, in 10 ms scheduler ticks. A task may run this
/// many consecutive ticks WITHOUT voluntarily blocking before it is deemed a
/// runaway (infinite loop / livelock) and terminated. 500 ticks = 5 s of
/// uninterrupted CPU — far beyond any cooperative or real-time cell, which block
/// (Recv/Send/Sleep) every iteration — so legitimate work never trips it. The
/// budget is kernel-owned; a cell cannot extend its own.
const WATCHDOG_BUDGET_TICKS: u32 = 500;

/// CPU-monopoly *warning* threshold (80% of the kill budget). An RT cell that crosses
/// this without yielding gets a one-shot `RtCpuOverrun` audit event — an early signal
/// that it is trending toward the hard watchdog kill, so an operator/log analysis can
/// catch a degrading RT loop before it is terminated. Observability only.
const WATCHDOG_WARN_TICKS: u32 = WATCHDOG_BUDGET_TICKS * 4 / 5;

/// Sentinel recorded as the "scause" in a `CellFault` audit entry for a watchdog
/// kill, to distinguish it from a real hardware trap.
const WATCHDOG_SCAUSE: u32 = 0x0000_DEAD;

/// Death-notification subscriptions: `watched_tid → [watcher_tid, …]`.
///
/// A watcher (a `SpawnCap` holder, e.g. a supervisor) registers via the
/// `NotifyOnExit` syscall; `exit_task` delivers to each watcher when the watched
/// task dies (wakes a parked `Recv`, or queues onto `Task::pending_deaths` if the
/// watcher is busy). One-shot: the subscription is removed on delivery, so a
/// supervisor re-registers for the respawned child.
///
/// Lock order: only ever locked while already holding (or after releasing)
/// SCHEDULER — never SUBSCRIBERS-then-SCHEDULER — to avoid deadlock.
static DEATH_SUBSCRIBERS: crate::sync::Spinlock<BTreeMap<usize, Vec<usize>>> =
    crate::sync::Spinlock::new(BTreeMap::new());

/// Register `watcher` to be notified when `watched` exits or faults.
pub fn subscribe_death(watched: usize, watcher: usize) {
    DEATH_SUBSCRIBERS.lock().entry(watched).or_default().push(watcher);
}

/// Central task table (Hubris-like).
///
/// Ready queues and current_task_id are now PER-HART in `ViHartLocal::ready`
/// and `ViHartLocal::current_task_id` (Phase 03).  This struct keeps only the
/// shared state that requires the global SCHEDULER lock: the task table itself,
/// the zombie list, and the next-id counter.
pub struct Scheduler {
    pub tasks: BTreeMap<usize, Box<Task>>,
    pub zombies: Vec<Box<Task>>,
    pub next_task_id: usize,
    /// Task IDs whose grant pages must be reaped outside the SCHEDULER lock.
    ///
    /// Watchdog kill paths push here instead of calling reap_grants_for_task directly,
    /// because free_grant_pages acquires KERNEL_ROOT and FRAME_ALLOCATOR while the
    /// watchdog runs inside SCHEDULER — inverting the documented lock order.
    /// yield_cpu() drains this list after dropping SCHEDULER, matching the zombie-reaper pattern.
    pub(super) pending_grant_reap: Vec<usize>,
    pub last_global_sweep_tick: usize,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            tasks: BTreeMap::new(),
            zombies: Vec::new(),
            next_task_id: 1,
            pending_grant_reap: Vec::new(),
            last_global_sweep_tick: 0,
        }
    }

    /// Push task `id` onto the CALLING hart's local ready queue.
    ///
    /// Returns the priority level used so callers can optionally call
    /// `pend_preempt_if_needed(priority)` to trigger zero-latency RT preemption.
    ///
    /// Call while holding SCHEDULER (lock order: SCHEDULER → per-hart ready).
    pub fn push_ready(&mut self, id: usize) -> u8 {
        let priority = self.tasks.get(&id)
            .map(|t| t.priority)
            .unwrap_or(api::TaskPriority::Normal as u8);
        // RT tasks target the dedicated RT hart when it is online; fall back to
        // the current hart on single-hart systems (e.g. QEMU without -smp 2).
        let target_hart = if priority >= api::TaskPriority::RealTime as u8
            && crate::task::smp::is_rt_hart_online()
        {
            crate::task::smp::HART_RT
        } else {
            super::hart_local::current_hart_id()
        };
        super::hart_local::ready::push_on_hart(target_hart, id, priority);
        priority
    }

    /// Pend an S-mode software interrupt if `new_priority` exceeds the current
    /// running task's priority.
    ///
    /// Call this after any syscall that transitions a task from blocked → Ready
    /// so that a newly-runnable RealTime cell preempts a Normal/Background cell
    /// within the same syscall return, rather than waiting for the next timer tick.
    ///
    /// The interrupt fires when the trap handler returns via `sret` and
    /// `sstatus.SIE` is restored by hardware.
    #[cfg(target_arch = "riscv64")]
    pub fn pend_preempt_if_needed(&self, new_priority: u8) {
        let hart_id = super::hart_local::current_hart_id();
        let current_tid = super::hart_local::ready::current_task_id_for(hart_id);
        let current_priority = if current_tid > 0 {
            self.tasks.get(&current_tid).map(|t| t.priority).unwrap_or(0)
        } else { 0 };

        if new_priority > current_priority {
            // RT tasks land on HART_RT when online; fall back to current hart on single-hart systems.
            let target_hart = if new_priority >= api::TaskPriority::RealTime as u8
                && crate::task::smp::is_rt_hart_online()
            {
                crate::task::smp::HART_RT
            } else {
                hart_id
            };
            if target_hart == hart_id {
                // SAFETY: csrsi on sip.SSIP is permitted from S-mode (RISC-V priv spec §4.1.3).
                // The interrupt fires after sret restores sstatus.SIE.
                unsafe { core::arch::asm!("csrsi sip, 0x2") };
            } else {
                // Cross-hart IPI: SSIP fires on the target hart's next interrupt check.
                let _ = hal::common::sbi::sbi_send_ipi(1 << target_hart, 0);
            }
        }
    }

    #[cfg(not(target_arch = "riscv64"))]
    pub fn pend_preempt_if_needed(&self, _new_priority: u8) {
        // No-op on non-riscv64 targets.
    }

    pub fn spawn(
        &mut self,
        name: &str,
        cell_id: CellId,
        allowed_drivers: alloc::vec::Vec<usize>,
    ) -> usize {
        let mut task = Box::new(Task::new(self.next_task_id, cell_id, name, allowed_drivers));
        task.state = TaskState::Ready;
        let id = task.id;

        // Stack Size: 8 Frames (32KB)
        use crate::task::STACK_PAGES as STACK_FRAMES;

        // Allocate Kernel Stack
        let kstack = crate::task::stack::Stack::new_kernel(STACK_FRAMES).expect("OOM Stack");

        // Stack grows DOWN. Top is at end of region.
        let stack_top = kstack.top;
        let stack_base = kstack.base;

        // Zero the usable stack pages. Skip the guard frame at `stack_base` (it is
        // unmapped — a write there faults); the usable region starts one page above
        // base and spans exactly STACK_FRAMES pages. (The old code zeroed from base,
        // which clobbered the guard AND missed the top usable page.)
        // SAFETY: we own these freshly-allocated, mapped frames exclusively.
        unsafe {
            core::ptr::write_bytes(
                (stack_base + crate::memory::paging::PAGE_SIZE) as *mut u8,
                0,
                STACK_FRAMES * crate::memory::paging::PAGE_SIZE,
            );
        }

        let entry = task_entry_point as *const () as usize;
        let (gp, tp) = crate::task::get_kernel_gp_tp();

        // Allocate User Stack
        let ustack = crate::task::stack::Stack::new_user(STACK_FRAMES).expect("OOM User Stack");
        let ustack_top = ustack.top;

        task.context.sp = stack_top as _;
        task.trap_frame.sepc = entry as _;
        task.trap_frame.sstatus = 0x20_u64 as _;  // SPIE enabled, SPP=0 (User Mode)
        task.trap_frame.regs[2] = ustack_top as _; // sp = x2
        #[cfg(target_arch = "riscv64")]
        { task.context.ra = entry; task.context.gp = gp; task.context.tp = tp; }
        #[cfg(target_arch = "aarch64")]
        { task.context.x30 = entry as u64; }
        #[cfg(target_arch = "x86_64")]
        { task.context.rip = entry as u64;
          task.context.kernel_trap_sp = stack_top as u64; }
        task.kernel_stack = Some(kstack);
        task.user_stack = Some(ustack);

        info!(
            "Task '{}' (ID {}): Stack 0x{:X}-0x{:X}, Entry 0x{:X}",
            name, id, stack_base, stack_top, entry
        );

        self.tasks.insert(id, task);
        self.push_ready(id);
        self.next_task_id += 1;
        id
    }

    pub fn spawn_thread(
        &mut self,
        name: &str,
        cell_id: CellId,
        allowed_drivers: alloc::vec::Vec<usize>,
        entry: usize,
        arg: usize,
    ) -> usize {
        let mut task = Box::new(Task::new(self.next_task_id, cell_id, name, allowed_drivers));
        task.state = TaskState::Ready;
        let id = task.id;

        use crate::task::STACK_PAGES as STACK_FRAMES;

        // Allocate Kernel Stack
        let kstack = crate::task::stack::Stack::new_kernel(STACK_FRAMES).expect("OOM Stack");

        let stack_top = kstack.top;
        let stack_base = kstack.base;

        // SAFETY: We own the allocated stack memory exclusively. The pointer is valid.
        // Setting up task context with valid register values for thread initialization.
        unsafe {
            // Skip the guard frame at `stack_base` (unmapped); zero only the
            // STACK_FRAMES usable pages that begin one page above base.
            core::ptr::write_bytes(
                (stack_base + crate::memory::paging::PAGE_SIZE) as *mut u8,
                0,
                STACK_FRAMES * crate::memory::paging::PAGE_SIZE,
            );

            let (gp, tp) = crate::task::get_kernel_gp_tp();
            let trampoline = crate::hal::arch::thread_trampoline as *const () as usize;

            task.context.sp = stack_top as _;
            task.trap_frame.sepc = trampoline as _;
            task.trap_frame.sstatus = 0x120;
            #[cfg(target_arch = "riscv64")]
            { task.context.ra = trampoline; task.context.s0 = arg; task.context.s1 = entry;
              task.context.gp = gp; task.context.tp = tp; }
            #[cfg(target_arch = "riscv32")]
            { task.context.ra = trampoline as u32; task.context.s0 = arg as u32;
              task.context.s1 = entry as u32; task.context.gp = gp as u32;
              task.context.tp = tp as u32; }
            #[cfg(target_arch = "aarch64")]
            { task.context.x30 = trampoline as u64;
              task.context.x19 = arg as u64;
              task.context.x20 = entry as u64; }
            #[cfg(target_arch = "x86_64")]
            {
                // thread_trampoline reads entry from RBX and arg from R12.
                // CpuContext::switch restores all callee-saved fields before
                // jumping to context.rip, so store entry/arg there.
                task.context.rip = trampoline as u64;
                task.context.rbx = entry as u64;    // thread body fn ptr
                task.context.r12 = arg as u64;      // argument
                task.context.kernel_trap_sp = stack_top as u64;
            }
            task.kernel_stack = Some(kstack);

            info!(
                "Thread '{}' (ID {}): Stack 0x{:X}-0x{:X}, Entry 0x{:X}, Arg 0x{:X}",
                name, id, stack_base, stack_top, entry, arg
            );
        }

        self.tasks.insert(id, task);
        self.push_ready(id);
        self.next_task_id += 1;
        id
    }

    /// Reap a task: move it to the zombie list, purge ready queues, unblock
    /// senders stuck on it, and wake any `Wait`-ers with `exit_reason`.
    ///
    /// `exit_reason` is delivered to waiters as their `reply_value` — the exit
    /// code for a clean `Exit`, or `usize::MAX` for a fault / force-kill.
    /// Centralizing the waiter-wake here is the contract that ALL death paths
    /// (clean `Exit`, `ForceExit`, AND hardware faults) notify waiters uniformly;
    /// the fault path previously skipped it, so `Wait(tid)` hung forever when the
    /// target died by fault.
    pub fn exit_task(&mut self, tid: usize, exit_reason: usize) {
        info!("Task {} exiting (reason={:#x})...", tid, exit_reason);

        // Capture waiters BEFORE the task is removed from the table.
        let waiters: Vec<usize> = self
            .tasks
            .get_mut(&tid)
            .map(|t| core::mem::take(&mut t.waiters))
            .unwrap_or_default();

        // Free the dying cell's address space NOW (unmap its segment VAs) so a
        // respawn can reuse the fixed VA and the load-time overwrite guard only
        // ever sees LIVE cells' mappings. Frames are freed lazily at reap.
        if let Some(t) = self.tasks.get(&tid) {
            if let Some(seg) = &t.segment_mem {
                seg.eager_unmap();
            }
        }

        // Async-future safety: a task may die while Polling, holding a `pending_future`
        // that captured a raw pointer into THIS cell's buffer (see fat.rs::read_async).
        // Removing it from `self.tasks` here — BEFORE its frames are freed at reap — takes
        // it out of the scheduler poll set (the loop iterates `self.tasks`, gated on
        // `state == Polling`), so the dangling buffer write can never execute. The future
        // itself is dropped at reap (outside the SCHEDULER lock) without touching the
        // buffer, because the inner read is synchronous (no DMA outlives the future).
        // INVARIANT for future work: if a real async-DMA driver lands (the fat.rs TODO) or
        // the kernel goes SMP, hardware could write into freed frames — add a descriptor
        // cancel / frame-unpin point HERE before the frames are reclaimed.
        if let Some(task) = self.tasks.remove(&tid) {
            self.zombies.push(task);
        }

        // Service-registry cleanup: drop any well-known service_id that pointed at this
        // tid, so a client lookup in the death→respawn window returns "none" (and retries)
        // rather than a dead provider. The supervisor re-registers the replacement's tid.
        // Locks only REGISTRY (a leaf), safe under the SCHEDULER lock.
        crate::cell::service_registry::clear_tid(tid);

        // Input-service registration cleanup: prevent the kernel poll path from pushing
        // events to a dead/reused TID. Supervisor re-registers after respawn.
        crate::task::drivers::virtio_input::clear_input_cell_if(tid);

        // Remove from every hart's ready queue if present.
        super::hart_local::ready::remove_from_all(tid);

        // Best-effort IPC cleanup: unblock tasks stuck sending to the dead task,
        // and clear stale current_caller references.  Does not handle multi-hop
        // chains — those require a full state-machine audit (future work).
        let mut to_wake = Vec::new();
        for (id, task) in self.tasks.iter_mut() {
            if let TaskState::Sending { target, .. } = task.state {
                if target == tid {
                    task.state = TaskState::Ready;
                    task.trap_frame.regs[10] = usize::MAX as _; // error return: target gone
                    to_wake.push(*id);
                }
            }
            if task.current_caller == Some(tid) {
                task.current_caller = None;
            }
        }
        for id in to_wake {
            self.push_ready(id);
        }

        // Wake tasks blocked on Wait(tid).  Last use of `w` ends its borrow of
        // self.tasks before push_ready re-borrows self (NLL) — mirrors the
        // former in-handler pattern, now the single source of truth.
        for wid in waiters {
            if let Some(w) = self.tasks.get_mut(&wid) {
                w.state = TaskState::Ready;
                w.reply_value = Some(exit_reason);
                self.push_ready(wid);
            }
        }

        // Deliver NotifyOnExit death notifications. One-shot: the subscription is
        // removed here. Wake a watcher parked in Recv (its Recv returns
        // current_caller = this dead tid), else queue onto pending_deaths so the
        // watcher gets it on its next Recv (covers a death during respawn).
        let watchers = DEATH_SUBSCRIBERS.lock().remove(&tid).unwrap_or_default();
        let mut woken_watchers = Vec::new();
        for w in watchers {
            if let Some(wt) = self.tasks.get_mut(&w) {
                if matches!(wt.state, TaskState::Recv { .. }) {
                    // Stash the exit reason for delivery as the recv payload (NotifyOnExit
                    // contract). The actual buffer write happens when the watcher's Recv
                    // RESUMES, in the watcher's own syscall context — writing a USER buffer
                    // from here (the trap/fault context) faults (S-mode store to a U page,
                    // SSTATUS.SUM not set).
                    wt.current_caller = Some(tid);
                    wt.pending_exit_reason = Some(exit_reason);
                    wt.state = TaskState::Ready;
                    woken_watchers.push(w);
                } else {
                    wt.pending_deaths.push((tid, exit_reason));
                }
            }
        }
        for w in woken_watchers {
            self.push_ready(w);
        }
    }

    /// Remove and return zombies that have already been switched away from — every
    /// zombie except the one still set as `current_task_id` (whose context is about
    /// to be used for the outgoing half of the next switch, so it must stay valid).
    ///
    /// The caller MUST drop the returned tasks OUTSIDE the SCHEDULER lock: dropping
    /// a `Box<Task>` runs `Stack::drop`, which locks `FRAME_ALLOCATOR` and unmaps
    /// via `KERNEL_ROOT`; doing that while holding `SCHEDULER` would invert the lock
    /// order. Returning the tasks (cheap pointer moves) keeps the lock window tiny.
    ///
    /// This is what actually frees a dead cell's kernel + user stack frames (the
    /// largest per-cell allocation) — without it, zombies accumulate forever and
    /// `Stack::drop` never runs (every cell death leaked its stacks).
    pub fn take_reapable_zombies(&mut self) -> Vec<Box<super::tcb::Task>> {
        if self.zombies.is_empty() { return Vec::new(); }
        let mut keep = Vec::new();
        let mut reap = Vec::new();
        for z in core::mem::take(&mut self.zombies) {
            // A zombie is reapable only if NO hart is currently context-switching
            // through its saved Context.  Check all harts' current_task_id.
            if super::hart_local::ready::any_hart_running(z.id) {
                keep.push(z);
            } else {
                reap.push(z);
            }
        }
        self.zombies = keep;
        reap
    }

    /// Take task IDs whose grant pages must be freed outside the SCHEDULER lock.
    ///
    /// The caller MUST call reap_grants_for_task for each returned ID OUTSIDE the lock —
    /// free_grant_pages locks KERNEL_ROOT and FRAME_ALLOCATOR; holding SCHEDULER inverts order.
    pub fn take_pending_grant_reap(&mut self) -> Vec<usize> {
        core::mem::take(&mut self.pending_grant_reap)
    }

    /// Picks the next task to run on `hart_id` and returns pointers for context switch.
    ///
    /// Hart 0 also runs the global sweep (timer wakes, heartbeat, async-poll, watchdog).
    /// Other harts only do the per-hart pick + work stealing.
    ///
    /// Returns: Option<(current_context_ptr, next_context_ptr)>
    pub fn pick_next(
        &mut self,
        hart_id: usize,
    ) -> Option<(
        *mut crate::hal::arch::Context,
        *const crate::hal::arch::Context,
    )> {
        let now = crate::task::system_ticks();
        // Global sweep (timer wakes, heartbeat, async-poll, watchdog) runs on hart 0 only
        // to prevent double-wake races on multihart setups.
        if hart_id != 0 { return self.pick_next_local(hart_id, now); }

        let time_advanced = now > self.last_global_sweep_tick;
        let events_pending = crate::task::waker::has_any_pending();

        if time_advanced || events_pending {
            if time_advanced {
                self.last_global_sweep_tick = now;
            }

            // 1. Wake tasks whose deadline elapsed: Sleeping (timer) and RecvTimeout
            //    (a Recv with a deadline). Without the RecvTimeout sweep a cell that
            //    RecvTimeout's a peer that never replies would block forever — the
            //    infinite-block-on-dead-peer hazard. Deadlines are absolute
            //    `system_ticks` (the dispatch stores `system_ticks() + timeout`).
            let mut waking_tasks = VecDeque::new();
            for (id, task) in self.tasks.iter_mut() {
                let mut should_wake = false;
                let mut timed_out = false;
                match &task.state {
                    TaskState::Sleeping { until } => {
                        if now >= *until {
                            should_wake = true;
                        }
                    }
                    TaskState::Recv { deadline: Some(d), .. } => {
                        // `deadline` is u64 (mtime-domain field); `now` is usize system
                        // ticks. On rv64 usize == u64, so the cast is lossless.
                        if now as u64 >= *d {
                            should_wake = true;
                            timed_out = true;
                        }
                    }
                    TaskState::WaitEvent { mask, deadline } => {
                        let fired = super::waker::consume_pending(*mask);
                        if fired != 0 {
                            // Return fired mask as the syscall result.
                            task.trap_frame.regs[10] = fired as usize;
                            should_wake = true;
                        } else if deadline.map(|d| now as u64 >= d).unwrap_or(false) {
                            task.trap_frame.regs[10] = 0; // timeout — return 0
                            should_wake = true;
                            timed_out = true;
                        }
                    }
                    _ => {}
                }
                if should_wake {
                    // ostd `sys_recv_timeout` returns Ok(0) on timeout; the syscall
                    // return register is regs[10], restored by sret when the task runs.
                    if timed_out {
                        task.trap_frame.regs[10] = 0;
                        task.deadline_misses = task.deadline_misses.saturating_add(1);
                        // Observability: an RT cell whose awaited message missed its deadline
                        // is a missed control-loop cycle — record it (no enforcement). Gated to
                        // RT priority so the safety-timeout use on Normal cells stays quiet.
                        if task.priority >= api::TaskPriority::RealTime as u8 {
                            crate::audit::log_event(
                                crate::audit::AuditEvent::RtDeadlineMiss,
                                &crate::audit::encode_u32x2(task.cell_id.0 as u32, task.deadline_misses),
                            );
                        }
                    }
                    task.state = TaskState::Ready;
                    waking_tasks.push_back(*id);
                }
            }
            for id in waking_tasks {
                self.push_ready(id);
            }

            // 1b. Heartbeat liveness sweep: terminate any cell that opted into heartbeating
            //     but missed its deadline — a SILENT hang (deadlock / stuck loop) that the
            //     CPU-monopoly watchdog cannot see (that only fires on RT compute hogs). The
            //     death flows through the normal path so the supervisor restarts it. Collect
            //     first, then `exit_task` outside the iteration (it mutates self.tasks).
            let mut hung: Vec<(usize, u64)> = Vec::new();
            for (id, task) in self.tasks.iter() {
                if let Some(d) = task.heartbeat_deadline {
                    if now as u64 >= d {
                        hung.push((*id, task.cell_id.0));
                    }
                }
            }
            for (tid, cell_raw) in hung {
                log::error!(
                    "[heartbeat] task {} (cell {}) missed liveness deadline — terminating (hung)",
                    tid, cell_raw
                );
                crate::audit::log_event(
                    crate::audit::AuditEvent::CellHung,
                    &crate::audit::encode_u32x2(cell_raw as u32, tid as u32),
                );
                // Release resources the hung cell owned (each locks its own state, not
                // SCHEDULER, so they are safe to call inline here — mirrors the watchdog kill).
                crate::fast_ipc::clear_vfs_if_cell(cell_raw as usize);
                crate::memory::cell_quota::deregister(CellId(cell_raw));
                crate::resource_registry::release_for(CellId(cell_raw));
                // Grant reap deferred: free_grant_pages acquires KERNEL_ROOT + FRAME_ALLOCATOR,
                // which must not be held under SCHEDULER. yield_cpu() drains this list after unlock.
                self.pending_grant_reap.push(tid);
                self.exit_task(tid, usize::MAX);
                // If this hart was running the hung task, clear its attribution.
                let hart_id = super::hart_local::current_hart_id();
                if super::hart_local::ready::current_task_id_for(hart_id) == tid {
                    super::hart_local::set_current_cell_id(0);
                    super::hart_local::ready::set_current_task_id(hart_id, 0);
                }
            }
        }

        // 2. Poll Async Tasks
        let has_polling = self.tasks.values().any(|t| t.state == TaskState::Polling);
        if has_polling {
            let mut polled_tasks = Vec::new();
            let waker = dummy_waker();
            let mut cx = Context::from_waker(&waker);

            // Iterate keys to avoid borrow check issues
            let keys: Vec<usize> = self.tasks.keys().cloned().collect();
            for id in keys {
                if let Some(task) = self.tasks.get_mut(&id) {
                    if task.state == TaskState::Polling {
                        if let Some(ref mut future_enum) = task.pending_future {
                            match future_enum {
                                SyscallFuture::FileRead(fd, future) => {
                                    // Poll the future
                                    match future.as_mut().poll(&mut cx) {
                                        Poll::Ready((file, res)) => {
                                            // Restore file handle
                                            // file is Box<dyn ViFile>
                                            task.open_files.insert(*fd, FileHandle::new(file));

                                            // Set return value (a0 / regs[10])
                                            task.trap_frame.regs[10] = res.unwrap_or(0) as _; // TODO: Handle Error Properly (negative?)


                                        // Wake task
                                        task.state = TaskState::Ready;
                                        task.pending_future = None;
                                        polled_tasks.push(id);
                                    }
                                    Poll::Pending => {
                                        // Still waiting
                                    } //
                                }
                            }
                        }
                    }
                    }
                }
            }
            for id in polled_tasks {
                self.push_ready(id);
            }
        }

        // After global sweep, fall through to per-hart pick.
        self.pick_next_local(hart_id, now)
    }

    /// Per-hart task selection: watchdog on current task, then pop from local queue
    /// (with work-stealing fallback).  Called by `pick_next` for both hart 0
    /// (after global sweep) and all other harts.
    fn pick_next_local(
        &mut self,
        hart_id: usize,
        _now: usize,
    ) -> Option<(
        *mut crate::hal::arch::Context,
        *const crate::hal::arch::Context,
    )> {
        use super::hart_local::ready as rl;

        // 3. Decide if the current task yields, and run the CPU-monopoly watchdog.
        let current_id_raw = rl::current_task_id_for(hart_id);
        let current_id: Option<usize> = if current_id_raw > 0 { Some(current_id_raw) } else { None };
        if let Some(cid) = current_id {
            enum WdAction { None, Requeue, Kill(u64) }
            let mut action = WdAction::None;
            if let Some(task) = self.tasks.get_mut(&cid) {
                if task.state == TaskState::Running {
                    // Only RealTime-priority tasks can livelock the system.
                    if task.priority >= api::TaskPriority::RealTime as u8 {
                        task.run_ticks = task.run_ticks.saturating_add(1);
                        if task.run_ticks >= WATCHDOG_WARN_TICKS && !task.rt_overrun_warned {
                            task.rt_overrun_warned = true;
                            crate::audit::log_event(
                                crate::audit::AuditEvent::RtCpuOverrun,
                                &crate::audit::encode_u32x2(task.cell_id.0 as u32, task.run_ticks),
                            );
                        }
                        if task.run_ticks > WATCHDOG_BUDGET_TICKS {
                            action = WdAction::Kill(task.cell_id.0);
                        } else {
                            task.state = TaskState::Ready;
                            action = WdAction::Requeue;
                        }
                    } else {
                        task.state = TaskState::Ready;
                        action = WdAction::Requeue;
                    }
                } else {
                    task.run_ticks = 0;
                    task.rt_overrun_warned = false;
                }
            }
            match action {
                WdAction::Requeue => { self.push_ready(cid); }
                WdAction::Kill(cell_raw) => {
                    log::error!(
                        "[watchdog] task {} (cell {}) monopolized CPU >{} ticks (~{}s) — terminating",
                        cid, cell_raw, WATCHDOG_BUDGET_TICKS, WATCHDOG_BUDGET_TICKS / 100
                    );
                    crate::audit::log_event(
                        crate::audit::AuditEvent::CellFault,
                        &crate::audit::encode_u32x2(cell_raw as u32, WATCHDOG_SCAUSE),
                    );
                    crate::fast_ipc::clear_vfs_if_cell(cell_raw as usize);
                    crate::memory::cell_quota::deregister(CellId(cell_raw));
                    crate::resource_registry::release_for(CellId(cell_raw));
                    // Grant reap deferred: free_grant_pages acquires KERNEL_ROOT + FRAME_ALLOCATOR,
                    // which must not be held under SCHEDULER. yield_cpu() drains this list after unlock.
                    self.pending_grant_reap.push(cid);
                    self.exit_task(cid, usize::MAX);
                    super::hart_local::set_current_cell_id(0);
                    rl::set_current_task_id(hart_id, 0);
                }
                WdAction::None => {}
            }
        }

        // 4. Get next task: local queue first, then work-steal from busiest other hart.
        let next_id = rl::pick_local(hart_id).or_else(|| {
            super::hart_local::ready::steal_from_busiest(hart_id);
            rl::pick_local(hart_id)
        });

        if let Some(nid) = next_id {
            if let Some(next_task) = self.tasks.get_mut(&nid) {
                next_task.state = TaskState::Running;
                super::hart_local::set_current_cell_id(next_task.cell_id.0 as usize);
            }
            if Some(nid) == current_id {
                rl::set_current_task_id(hart_id, nid);
                return None; // No switch needed
            }
            // SAFETY: Box<Task> pins the Task on the heap; pointer is valid until reap.
            let next_ctx: *const crate::hal::arch::Context = self.tasks
                .get(&nid).map(|t| &t.context as *const _)
                .unwrap_or(core::ptr::null());
            rl::set_current_task_id(hart_id, nid);

            if let Some(cid) = current_id {
                let curr_ctx: *mut crate::hal::arch::Context =
                    if let Some(t) = self.tasks.get_mut(&cid) { &mut t.context as *mut _ }
                    else if let Some(t) = self.zombies.iter_mut().find(|t| t.id == cid) { &mut t.context as *mut _ }
                    else { core::ptr::null_mut() };
                if !curr_ctx.is_null() && !next_ctx.is_null() {
                    return Some((curr_ctx, next_ctx));
                }
            } else if !next_ctx.is_null() {
                return Some((core::ptr::null_mut(), next_ctx)); // first switch from boot context
            }
        } else {
            // No ready tasks.
            if let Some(cid) = current_id {
                if self.zombies.iter().any(|t| t.id == cid) {
                    // Zombie with no successor: switch to the idle boot context so
                    // it can be reaped without holding the SCHEDULER lock.
                    let curr_ctx = self.zombies.iter_mut()
                        .find(|t| t.id == cid)
                        .map(|t| &mut t.context as *mut _);
                    if let Some(c) = curr_ctx {
                        rl::set_current_task_id(hart_id, 0);
                        return Some((c, core::ptr::null()));
                    }
                } else if let Some(task) = self.tasks.get_mut(&cid) {
                    // Live blocked task with no peer ready to run.  Suspend it
                    // cleanly by switching to the idle (boot) context so the CPU
                    // can enter WFI and wake when a real event unblocks someone.
                    //
                    // Without this switch, yield_cpu returns without a context
                    // change, the SVC handler gets stale Ok(0) results, and
                    // current_task_id is reset to 0 — causing every subsequent
                    // SVC to be denied.
                    //
                    // BOOT_CONTEXT is valid here: it is saved by __switch on the
                    // very first boot→task context switch, which always precedes
                    // any cell SVC.
                    let curr_ctx = &mut task.context as *mut _;
                    rl::set_current_task_id(hart_id, 0);
                    return Some((curr_ctx, core::ptr::null()));
                } else {
                    rl::set_current_task_id(hart_id, 0);
                }
            } else {
                rl::set_current_task_id(hart_id, 0);
            }
        }
        None
    }

    pub fn current_task_mut(&mut self) -> Option<&mut Task> {
        let tid = super::hart_local::ready::current_task_id_for(super::hart_local::current_hart_id());
        if tid > 0 { self.tasks.get_mut(&tid).map(|b| &mut **b) } else { None }
    }

    pub fn current_task_ref(&self) -> Option<&Task> {
        let tid = super::hart_local::ready::current_task_id_for(super::hart_local::current_hart_id());
        if tid > 0 { self.tasks.get(&tid).map(|b| &**b) } else { None }
    }

    pub fn has_ready_tasks(&self) -> bool {
        super::hart_local::ready::total_ready_count() > 0
    }
}

/// Default entry point for kernel tasks
#[no_mangle]
extern "C" fn task_entry_point() {
    // SAFETY: This is the entry point for new tasks. We need to:
    // 1. Force unlock the scheduler (safe because we're in a new task context)
    // 2. Initialize HAL for this task context
    // 3. Enable interrupts (safe because stack is properly set up)
    unsafe {
        crate::task::SCHEDULER.force_unlock();
        crate::hal::arch::init();
        // Enable Interrupts MANUALLY now that we're safe and stack is clean
        crate::hal::arch::enable_interrupts();
    }
    info!("Task started!");
    loop {
        for _ in 0..10_000_000 {
            core::hint::spin_loop();
        }
        info!("Task tick (ID: {})...", crate::task::current_task_id());
        crate::task::yield_cpu();
    }
}
