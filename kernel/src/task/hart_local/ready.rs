//! Per-hart ready-queue helpers for the Phase 03 work-stealing scheduler.
//!
//! Lock order: SCHEDULER (global, coarse) → per-hart ready lock (leaf).
//! `steal_from_busiest` holds only leaf locks — never SCHEDULER.
//!
//! RT tasks (priority ≥ RealTime) are never stolen; Phase 04 will hart-pin them.

use super::{HART_LOCALS, MAX_HARTS};
use alloc::collections::VecDeque;

const RT_PRIO: u8 = api::TaskPriority::RealTime as u8;

/// Push task `id` with `priority` onto `hart_id`'s local ready queue.
///
/// Call while holding SCHEDULER (lock order: SCHEDULER → ready).
pub fn push_on_hart(hart_id: usize, id: usize, priority: u8) {
    if hart_id < MAX_HARTS {
        HART_LOCALS[hart_id].ready.lock()
            .entry(priority)
            .or_insert_with(VecDeque::new)
            .push_back(id);
    }
}

/// Push task `id` with `priority` onto the CALLING hart's local queue.
pub fn push_on_current_hart(id: usize, priority: u8) {
    push_on_hart(super::current_hart_id(), id, priority);
}

/// Pop the highest-priority ready task from `hart_id`'s local queue.
/// Returns None if empty.  May be called without SCHEDULER.
pub fn pick_local(hart_id: usize) -> Option<usize> {
    if hart_id >= MAX_HARTS { return None; }
    let mut rq = HART_LOCALS[hart_id].ready.lock();
    for queue in rq.values_mut().rev() {
        if let Some(id) = queue.pop_front() { return Some(id); }
    }
    None
}

/// Remove task `id` from every hart's ready queue.
/// Call while holding SCHEDULER (lock order: SCHEDULER → ready).
pub fn remove_from_all(id: usize) {
    for h in 0..MAX_HARTS {
        let mut rq = HART_LOCALS[h].ready.lock();
        for queue in rq.values_mut() { queue.retain(|&x| x != id); }
    }
}

/// Total ready-task count summed across all harts.
pub fn total_ready_count() -> usize {
    (0..MAX_HARTS).map(|h| {
        HART_LOCALS[h].ready.lock().values().map(|q| q.len()).sum::<usize>()
    }).sum()
}

/// Current task ID on `hart_id`.  Returns 0 if idle.
#[inline(always)]
pub fn current_task_id_for(hart_id: usize) -> usize {
    if hart_id < MAX_HARTS {
        HART_LOCALS[hart_id].current_task_id.load(core::sync::atomic::Ordering::Acquire)
    } else { 0 }
}

/// Set the current task ID for `hart_id` (0 = idle).
#[inline(always)]
pub fn set_current_task_id(hart_id: usize, id: usize) {
    if hart_id < MAX_HARTS {
        HART_LOCALS[hart_id].current_task_id.store(id, core::sync::atomic::Ordering::Release);
    }
}

/// Returns true if any hart is currently running `task_id`.
pub fn any_hart_running(task_id: usize) -> bool {
    (0..MAX_HARTS).any(|h| {
        HART_LOCALS[h].current_task_id.load(core::sync::atomic::Ordering::Acquire) == task_id
    })
}

/// Move up to `ceil(stealable/2)` Normal/Background tasks from the busiest other
/// hart's queue into `thief`'s queue.  Never steals RT tasks.
///
/// Always locks hart 0 then hart 1 (ABBA-safe for MAX_HARTS=2).
pub fn steal_from_busiest(thief: usize) {
    if thief >= MAX_HARTS { return; }
    // Only 2 harts: victim is always the other one.
    let victim = 1 - thief;

    // Acquire in hart-id order to prevent ABBA deadlock.
    let mut g0 = HART_LOCALS[0].ready.lock();
    let mut g1 = HART_LOCALS[1].ready.lock();

    // Count stealable tasks on victim (Normal + Background only).
    let stealable: usize = if victim == 0 {
        g0.iter().filter(|(&p, _)| p < RT_PRIO).map(|(_, q)| q.len()).sum()
    } else {
        g1.iter().filter(|(&p, _)| p < RT_PRIO).map(|(_, q)| q.len()).sum()
    };
    if stealable == 0 { return; }
    let to_steal = (stealable / 2).max(1);

    // Move tasks, highest-priority first (Normal before Background).
    let mut stolen = 0;
    for p in (0..RT_PRIO).rev() {
        if stolen >= to_steal { break; }
        if thief == 0 {
            // victim=1(g1) → thief=0(g0)
            if let Some(vq) = g1.get_mut(&p) {
                while stolen < to_steal {
                    match vq.pop_front() {
                        Some(id) => { g0.entry(p).or_insert_with(VecDeque::new).push_back(id); stolen += 1; }
                        None => break,
                    }
                }
            }
        } else {
            // victim=0(g0) → thief=1(g1)
            if let Some(vq) = g0.get_mut(&p) {
                while stolen < to_steal {
                    match vq.pop_front() {
                        Some(id) => { g1.entry(p).or_insert_with(VecDeque::new).push_back(id); stolen += 1; }
                        None => break,
                    }
                }
            }
        }
    }
}
