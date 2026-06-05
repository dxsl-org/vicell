//! Unit tests for Scheduler and Task Logic
//!
//! Intended to be run manually or via a custom runner as kernel is no_std binary.

#![allow(dead_code)]

use crate::task::scheduler::Scheduler;
use crate::task::tcb::{Task, TaskState};
use alloc::vec::Vec;
use types::CellId;

/// Manual Test Runner
pub fn run_scheduler_tests() {
    log::info!("=== Scheduler Tests ===");
    test_scheduler_task_table();
    test_task_state_transitions();
    test_reply_value_storage();
    test_current_caller_tracking();
    test_lease_attributes();
    test_round_robin_scheduling();
    test_scheduler_current_task();
    test_multiple_tasks_ready_queue();
    // Phase 11 additions
    test_blocked_then_ready_transition();
    test_waiting_task_not_scheduled();
    test_task_state_recv_deadline();
    // Phase 25 additions
    test_rt_preempts_normal_priority();
    test_background_lower_than_normal_priority();
    test_same_priority_round_robin();
    log::info!("=== Scheduler Tests PASSED ===");
}

/// RealTime tasks must always be selected before Normal tasks.
fn test_rt_preempts_normal_priority() {
    let mut sched = Scheduler::new();

    let normal_id = sched.spawn("normal", CellId(0), Vec::new());
    let rt_id = sched.spawn("rt", CellId(0), Vec::new());

    // Set rt task to RealTime priority
    if let Some(t) = sched.tasks.get_mut(&rt_id) {
        t.priority = api::TaskPriority::RealTime as u8;
    }

    // pick_next must select the RealTime task first.
    sched.pick_next();
    assert_eq!(
        sched.current_task_id,
        Some(rt_id),
        "RealTime task must preempt Normal task"
    );
    log::info!("  [ok] RealTime preempts Normal");
    let _ = normal_id;
}

/// Background tasks must lose to Normal tasks.
fn test_background_lower_than_normal_priority() {
    let mut sched = Scheduler::new();

    let bg_id = sched.spawn("bg", CellId(0), Vec::new());
    let normal_id = sched.spawn("normal", CellId(0), Vec::new());

    if let Some(t) = sched.tasks.get_mut(&bg_id) {
        t.priority = api::TaskPriority::Background as u8;
    }

    sched.pick_next();
    assert_eq!(
        sched.current_task_id,
        Some(normal_id),
        "Normal task must run before Background task"
    );
    log::info!("  [ok] Background loses to Normal");
}

/// Tasks at the same priority level must share the CPU in FIFO order.
fn test_same_priority_round_robin() {
    let mut sched = Scheduler::new();

    let id1 = sched.spawn("a", CellId(0), Vec::new());
    let id2 = sched.spawn("b", CellId(0), Vec::new());
    let id3 = sched.spawn("c", CellId(0), Vec::new());

    // First pick: id1 (spawned first → front of queue)
    sched.pick_next();
    assert_eq!(sched.current_task_id, Some(id1));

    // Second pick: id2
    sched.pick_next();
    assert_eq!(sched.current_task_id, Some(id2));

    // Third pick: id3
    sched.pick_next();
    assert_eq!(sched.current_task_id, Some(id3));

    log::info!("  [ok] Same-priority tasks round-robin in FIFO order");
}

fn test_scheduler_task_table() {
    let mut sched = Scheduler::new();

    // Spawn a task
    let id = sched.spawn("test-task", CellId(0), Vec::new());

    // Verify task exists in table
    assert!(sched.tasks.contains_key(&id));
    assert_eq!(sched.tasks.get(&id).unwrap().name, "test-task");

    // Verify task is in ready queue
    assert_eq!(sched.ready_count(), 1);
}

fn test_task_state_transitions() {
    // Note: Task::new expects allowed_drivers
    let mut task = Task::new(1, CellId(0), "test", Vec::new());

    // Initial state
    assert_eq!(task.state, TaskState::Ready);

    // Transition to Sending
    task.state = TaskState::Sending {
        target: 2,
        msg_ptr: 0x1000,
        msg_len: 64,
    };

    match task.state {
        TaskState::Sending { target, .. } => assert_eq!(target, 2),
        _ => panic!("Expected Sending state"),
    }

    // Transition to Recv
    task.state = TaskState::Recv {
        mask: 0,
        buf_ptr: 0x2000,
        buf_len: 128,
        deadline: None,
    };

    match task.state {
        TaskState::Recv { buf_len, .. } => assert_eq!(buf_len, 128),
        _ => panic!("Expected Recv state"),
    }
}

fn test_reply_value_storage() {
    let mut task = Task::new(1, CellId(0), "test", Vec::new());

    // Initially no reply value
    assert_eq!(task.reply_value, None);

    // Set reply value
    task.reply_value = Some(0xDEADBEEF);

    // Verify
    assert_eq!(task.reply_value, Some(0xDEADBEEF));
}

fn test_current_caller_tracking() {
    let mut task = Task::new(1, CellId(0), "server", Vec::new());

    // Initially no caller
    assert_eq!(task.current_caller, None);

    // Client 5 sends to us
    task.current_caller = Some(5);

    // Verify
    assert_eq!(task.current_caller, Some(5));

    // After reply, clear
    task.current_caller = None;
    assert_eq!(task.current_caller, None);
}

fn test_lease_attributes() {
    use crate::task::tcb::LeaseAttributes;

    let read_only = LeaseAttributes::READ;
    // let write_only = LeaseAttributes::WRITE; // Not used
    let read_write = LeaseAttributes(LeaseAttributes::READ.0 | LeaseAttributes::WRITE.0);

    // Test contains
    assert!(read_write.contains(LeaseAttributes::READ));
    assert!(read_write.contains(LeaseAttributes::WRITE));
    assert!(!read_only.contains(LeaseAttributes::WRITE));
}

fn test_round_robin_scheduling() {
    let mut sched = Scheduler::new();

    let id1 = sched.spawn("task1", CellId(0), Vec::new());
    let id2 = sched.spawn("task2", CellId(0), Vec::new());

    // pick_next -> should be task1
    sched.pick_next(); // Selects task1
    assert_eq!(sched.current_task_id, Some(id1));

    // pick_next -> should be task2 (Round Robin)
    // Note: pick_next() logic:
    // 1. If current (task1) is Running, force it to Ready locally?
    // In pick_next: "Decide if current task needs to yield".
    // "if task.state == TaskState::Running { task.state = TaskState::Ready; push_back }"
    // But spawn() sets state to Ready.
    // First pick_next sets task1 to Running.
    // Second pick_next sees task1 is Running. Moves it to Ready Queue end. Pop task2.
    sched.pick_next();
    assert_eq!(sched.current_task_id, Some(id2));

    // pick_next -> should be task1 again
    sched.pick_next();
    assert_eq!(sched.current_task_id, Some(id1));
}

fn test_scheduler_current_task() {
    let mut sched = Scheduler::new();

    // Initially no current task
    assert_eq!(sched.current_task_id, None);

    // Spawn and schedule
    let id = sched.spawn("test", CellId(0), Vec::new());
    
    // Sched::pick_next would be called by yield/interrupt.
    // We simulate it here.
    let _ = sched.pick_next();

    // Now should have current task (if pick_next selected it)
    assert_eq!(sched.current_task_id, Some(id));

    // Verify we can access it
    let task = sched.current_task_ref();
    assert!(task.is_some());
    assert_eq!(task.unwrap().id, id);
}

fn test_multiple_tasks_ready_queue() {
    let mut sched = Scheduler::new();

    // Spawn 3 tasks
    let id1 = sched.spawn("task1", CellId(0), Vec::new());
    let id2 = sched.spawn("task2", CellId(0), Vec::new());
    let id3 = sched.spawn("task3", CellId(0), Vec::new());

    // All should be in ready queue
    assert_eq!(sched.ready_count(), 3);

    // All should be in task table
    assert!(sched.tasks.contains_key(&id1));
    assert!(sched.tasks.contains_key(&id2));
    assert!(sched.tasks.contains_key(&id3));
}

// ─── Phase 11 additions ───────────────────────────────────────────────────────

/// A task in `Sending` state must be moved to `Ready` when the send is resolved,
/// and the scheduler must then pick it up on the next tick.
fn test_blocked_then_ready_transition() {
    let mut sched = Scheduler::new();
    let id = sched.spawn("blocked", CellId(0), Vec::new());

    // Simulate the task blocking on Send.
    if let Some(task) = sched.tasks.get_mut(&id) {
        task.state = TaskState::Sending { target: 99, msg_ptr: 0x1000, msg_len: 16 };
    }
    // Task should be removed from the ready queue.
    assert!(!sched.ready_queues.values().any(|q| q.contains(&id)), "blocked task should not be in ready queue");

    // Resolve: move back to Ready (simulates IPC delivery).
    if let Some(task) = sched.tasks.get_mut(&id) {
        task.state = TaskState::Ready;
    }
    sched.push_ready(id);

    // Now pick_next should select it.
    let picked = sched.pick_next();
    assert_eq!(sched.current_task_id, Some(id), "unblocked task should be scheduled next");
    log::info!("  [ok] blocked → Sending → Ready → scheduled");
}

/// A task in `Waiting` state (joined to another task) must never be scheduled.
fn test_waiting_task_not_scheduled() {
    let mut sched = Scheduler::new();
    let waiter = sched.spawn("waiter", CellId(0), Vec::new());
    let target = sched.spawn("target", CellId(0), Vec::new());

    // Put waiter into Waiting state.
    if let Some(task) = sched.tasks.get_mut(&waiter) {
        task.state = TaskState::Waiting { target };
        // Remove from ready queue (normally done by ipc_wait).
        for q in sched.ready_queues.values_mut() {
            q.retain(|&id| id != waiter);
        }
    }

    // Schedule: should pick `target`, not `waiter`.
    sched.pick_next();
    assert_eq!(sched.current_task_id, Some(target), "waiting task must not be selected");
    log::info!("  [ok] Waiting task not scheduled");
}

/// A Recv state with a deadline field must preserve the deadline through
/// state assignment and reading.
fn test_task_state_recv_deadline() {
    let mut task = Task::new(1, CellId(0), "test", Vec::new());

    // Assign Recv with a deadline.
    let dl: u64 = 999_999;
    task.state = TaskState::Recv { mask: 0, buf_ptr: 0x2000, buf_len: 256, deadline: Some(dl) };

    match task.state {
        TaskState::Recv { deadline: Some(d), buf_len, .. } => {
            assert_eq!(d, dl);
            assert_eq!(buf_len, 256);
        }
        _ => panic!("Expected Recv with deadline"),
    }

    // Assign without a deadline — must default to None.
    task.state = TaskState::Recv { mask: 0, buf_ptr: 0, buf_len: 0, deadline: None };
    match task.state {
        TaskState::Recv { deadline: None, .. } => {}
        _ => panic!("Expected Recv with no deadline"),
    }
    log::info!("  [ok] Recv deadline field preserved correctly");
}
