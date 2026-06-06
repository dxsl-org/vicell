use super::tcb::{FileHandle, SyscallFuture, Task, TaskState};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::task::{Context, Poll};
use log::info;
use types::*;

/// Cell ID currently executing on this hart.  0 = kernel itself (no quota limit).
/// Updated on every context switch so `QuotaAlloc` can attribute allocations correctly.
pub static CURRENT_CELL_ID: AtomicUsize = AtomicUsize::new(0);

/// Read the currently-executing cell ID (0 = kernel).
pub fn current_cell_id() -> usize {
    CURRENT_CELL_ID.load(Ordering::Relaxed)
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

/// Sentinel recorded as the "scause" in a `CellFault` audit entry for a watchdog
/// kill, to distinguish it from a real hardware trap.
const WATCHDOG_SCAUSE: u32 = 0x0000_DEAD;

/// Priority-aware Scheduler with Central Task Table (Hubris-like).
///
/// Three priority levels (Background=0, Normal=1, RealTime=2) are stored as
/// separate `VecDeque` queues keyed by `u8`.  `pop_ready()` always returns
/// from the highest non-empty level, giving O(1) selection for 3 levels.
pub struct Scheduler {
    pub tasks: BTreeMap<usize, Box<Task>>,
    pub zombies: Vec<Box<Task>>,
    /// Per-priority ready queues.  Key = priority `u8`; higher key = higher priority.
    pub ready_queues: BTreeMap<u8, VecDeque<usize>>,
    pub current_task_id: Option<usize>,
    pub next_task_id: usize,
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
            ready_queues: BTreeMap::new(),
            current_task_id: None,
            next_task_id: 1,
        }
    }

    /// Push task `id` onto the ready queue at its priority level.
    ///
    /// Returns the priority level used so callers can optionally call
    /// `pend_preempt_if_needed(priority)` to trigger zero-latency RT preemption.
    pub fn push_ready(&mut self, id: usize) -> u8 {
        let priority = self.tasks.get(&id)
            .map(|t| t.priority)
            .unwrap_or(api::TaskPriority::Normal as u8);
        self.ready_queues
            .entry(priority)
            .or_insert_with(VecDeque::new)
            .push_back(id);
        priority
    }

    /// Pop the task with the highest priority from the ready queues.
    ///
    /// Ties within the same priority level are broken by FIFO insertion order.
    pub fn pop_ready(&mut self) -> Option<usize> {
        for queue in self.ready_queues.values_mut().rev() {
            if let Some(id) = queue.pop_front() {
                return Some(id);
            }
        }
        None
    }

    /// Total number of tasks across all priority ready queues.
    pub fn ready_count(&self) -> usize {
        self.ready_queues.values().map(|q| q.len()).sum()
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
        let current_priority = self.current_task_id
            .and_then(|id| self.tasks.get(&id))
            .map(|t| t.priority)
            .unwrap_or(0);

        if new_priority > current_priority {
            // SAFETY: csrsi on sip.SSIP is permitted from S-mode (RISC-V priv spec §4.1.3).
            // The interrupt fires after sret restores sstatus.SIE.
            unsafe { core::arch::asm!("csrsi sip, 0x2") };
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

        task.context.sp = stack_top;
        task.context.ra = entry;
        task.trap_frame.sepc = entry;
        task.trap_frame.sstatus = 0x20; // 0x20 = SPIE enabled, SPP = 0 (User Mode)
        task.trap_frame.regs[2] = ustack_top; // sp = x2
        task.context.gp = gp;
        task.context.tp = tp;
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

            task.context.sp = stack_top;
            task.context.ra = trampoline;
            task.context.s0 = arg;
            task.context.s1 = entry;
            task.context.gp = gp;
            task.context.tp = tp;
            task.trap_frame.sepc = trampoline;
            task.trap_frame.sstatus = 0x120;
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

        if let Some(task) = self.tasks.remove(&tid) {
            self.zombies.push(task);
        }

        // Remove from ready queues if present
        for queue in self.ready_queues.values_mut() {
            queue.retain(|&id| id != tid);
        }

        // Best-effort IPC cleanup: unblock tasks stuck sending to the dead task,
        // and clear stale current_caller references.  Does not handle multi-hop
        // chains — those require a full state-machine audit (future work).
        let mut to_wake = Vec::new();
        for (id, task) in self.tasks.iter_mut() {
            if let TaskState::Sending { target, .. } = task.state {
                if target == tid {
                    task.state = TaskState::Ready;
                    task.trap_frame.regs[10] = usize::MAX; // error return: target gone
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
    }

    /// Picks the next task to run and returns pointers for context switch.
    /// Returns: Option<(current_context_ptr, next_context_ptr)>
    pub fn pick_next(
        &mut self,
    ) -> Option<(
        *mut crate::hal::arch::Context,
        *const crate::hal::arch::Context,
    )> {
        let now = crate::task::system_ticks();

        // 1. Wake up sleeping tasks
        let mut waking_tasks = VecDeque::new();
        for (id, task) in self.tasks.iter_mut() {
            let mut should_wake = false;
            if let TaskState::Sleeping { until } = &task.state {
                if now >= *until {
                    should_wake = true;
                }
            }
            if should_wake {
                task.state = TaskState::Ready;
                waking_tasks.push_back(*id);
            }
        }
        for id in waking_tasks {
            self.push_ready(id);
        }

        // 2. Poll Async Tasks
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
                                        task.trap_frame.regs[10] = res.unwrap_or(0); // TODO: Handle Error Properly (negative?)

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

        // 3. Decide if the current task yields, and run the CPU-monopoly watchdog.
        //    A task found Running here either was timer-preempted or yielded without
        //    blocking — either way it consumed this slice, so charge a run_tick. A
        //    task that voluntarily blocked set a non-Running state before yielding,
        //    so we reset its budget. Crossing WATCHDOG_BUDGET_TICKS means it never
        //    blocked — a runaway/livelock — so we terminate it (kernel survives).
        let current_id = self.current_task_id;
        if let Some(cid) = current_id {
            enum WdAction {
                None,
                Requeue,
                Kill(u64),
            }
            let mut action = WdAction::None;
            if let Some(task) = self.tasks.get_mut(&cid) {
                if task.state == TaskState::Running {
                    // Only RealTime-priority tasks can livelock the system: they
                    // always win pop_ready, so a pure-compute RT loop starves
                    // everyone. Normal/Background compute-heavy cells are fine —
                    // preemptive round-robin shares the CPU, so they cause no
                    // starvation and must NOT be killed (that would false-positive
                    // on legitimate heavy computation, e.g. a benchmark or sensor
                    // fusion loop). Combined with the syscall-entry reset, this only
                    // ever fires on an RT cell that runs pure compute (no syscalls)
                    // past the budget — a genuine RT runaway.
                    if task.priority >= api::TaskPriority::RealTime as u8 {
                        task.run_ticks = task.run_ticks.saturating_add(1);
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
                    // Voluntarily blocked (Recv/Sending/Sleeping/...) — not hogging.
                    task.run_ticks = 0;
                }
            }
            match action {
                WdAction::Requeue => {
                    self.push_ready(cid);
                }
                WdAction::Kill(cell_raw) => {
                    log::error!(
                        "[watchdog] task {} (cell {}) monopolized CPU >{} ticks (~{}s) — terminating",
                        cid,
                        cell_raw,
                        WATCHDOG_BUDGET_TICKS,
                        WATCHDOG_BUDGET_TICKS / 100
                    );
                    crate::audit::log_event(
                        crate::audit::AuditEvent::CellFault,
                        &crate::audit::encode_u32x2(cell_raw as u32, WATCHDOG_SCAUSE),
                    );
                    // Release resources the runaway owned (these lock their own
                    // state, not SCHEDULER, so they are safe to call inline here).
                    crate::fast_ipc::clear_vfs_if_cell(cell_raw as usize);
                    crate::memory::cell_quota::deregister(CellId(cell_raw));
                    // exit_task is a method on this already-locked Scheduler — no
                    // SCHEDULER re-lock. Moves the runaway to zombies + wakes its
                    // waiters; the pop_ready below then picks another ready task.
                    self.exit_task(cid, usize::MAX);
                    // Drop the dead cell's attribution; step 4 overwrites this if a
                    // next task is picked, else 0 (kernel) is correct for idle.
                    CURRENT_CELL_ID.store(0, Ordering::Relaxed);
                }
                WdAction::None => {}
            }
        }

        // 4. Get next task (highest-priority first; FIFO within same level)
        let next_id = self.pop_ready();

        if let Some(nid) = next_id {
            if let Some(next_task) = self.tasks.get_mut(&nid) {
                next_task.state = TaskState::Running;
                // Update CURRENT_CELL_ID so QuotaAlloc attributes allocations
                // to the correct Cell during this task's execution.
                CURRENT_CELL_ID.store(next_task.cell_id.0 as usize, Ordering::Relaxed);
            }

            if Some(nid) == current_id {
                self.current_task_id = Some(nid);
                return None; // No switch needed
            }

            // Get a raw pointer to next task's context. The Task lives inside Box<Task>
            // which is heap-allocated — its address is stable even if BTreeMap rebalances.
            // We drop the reference immediately (converting to *const) so the immutable
            // borrow does not alias the subsequent mutable borrow for curr_ctx.
            // SAFETY: Box<Task> keeps the Task on the heap. Pointer is valid for as long as
            // the Task remains in self.tasks or self.zombies (it is not removed until after
            // the context switch returns and the task is explicitly reaped).
            let next_ctx: *const crate::hal::arch::Context = self
                .tasks
                .get(&nid)
                .map(|t| &t.context as *const _)
                .unwrap_or(core::ptr::null());
            self.current_task_id = Some(nid);

            if let Some(cid) = current_id {
                let curr_ctx: *mut crate::hal::arch::Context =
                    if let Some(t) = self.tasks.get_mut(&cid) {
                        &mut t.context as *mut _
                    } else if let Some(t) = self.zombies.iter_mut().find(|t| t.id == cid) {
                        &mut t.context as *mut _
                    } else {
                        core::ptr::null_mut()
                    };

                if !curr_ctx.is_null() && !next_ctx.is_null() {
                    return Some((curr_ctx, next_ctx));
                }
            } else if !next_ctx.is_null() {
                // First switch from boot context
                return Some((core::ptr::null_mut(), next_ctx));
            }
        } else {
            // No ready tasks.
            // If we are currently running a zombie (exiting), we MUST switch to something. (Boot Context)
            if let Some(cid) = current_id {
                // Check if current is zombie
                let is_zombie = self.zombies.iter().any(|t| t.id == cid);
                if is_zombie {
                    // unsafe {
                        let curr_ctx = self
                            .zombies
                            .iter_mut()
                            .find(|t| t.id == cid)
                            .map(|t| &mut t.context as *mut _);
                        if let Some(c) = curr_ctx {
                            // Switch to NULL next (Boot Context)
                            self.current_task_id = None;
                            return Some((c, core::ptr::null()));
                        }
                    // }
                }
            }

            self.current_task_id = None;
        }

        None
    }

    pub fn current_task_mut(&mut self) -> Option<&mut Task> {
        self.current_task_id
            .and_then(|id| self.tasks.get_mut(&id).map(|b| &mut **b))
    }

    pub fn current_task_ref(&self) -> Option<&Task> {
        self.current_task_id
            .and_then(|id| self.tasks.get(&id).map(|b| &**b))
    }

    pub fn has_ready_tasks(&self) -> bool {
        self.ready_queues.values().any(|q| !q.is_empty())
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
