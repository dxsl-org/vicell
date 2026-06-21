//! SMP throughput scenarios — validate 2-hart work-stealing on QEMU.
//!
//! All spawn/notify/force_exit helpers run only in the orchestrator context
//! (which has SpawnCap).  Only `run_worker` is called from bench-probe.

use alloc::format;
use core::hint::black_box;
use ostd::{
    io::println,
    syscall::{
        sys_exit, sys_force_exit, sys_get_time, sys_notify_on_exit,
        sys_recv, sys_send, sys_set_spawn_args, sys_spawn_pinned, SyscallResult,
    },
    task::yield_now,
};
use api::task::TaskPriority;
use crate::framework::timer::NS_PER_TICK;

/// Iterations per worker run.  Calibrated for ≥1 ms on QEMU TCG.
pub const SMP_WORKER_ITERS: u64 = 500_000;

const PROBE_PATH: &str = "/bin/bench-probe";
const CAVEAT: &str = " [QEMU-TCG: 2 hart-threads; real-HW shows true 2× parallelism]";

/// Optimizer-resistant arithmetic workload shared by worker and orchestrator.
fn compute(iters: u64) -> u64 {
    let mut acc = 0u64;
    for i in 0..iters {
        acc = acc.wrapping_add(i.wrapping_mul(3));
    }
    black_box(acc)
}

/// `smp-worker` bench-probe role entry — runs compute, prints, exits.
pub fn run_worker() -> ! {
    let acc = compute(SMP_WORKER_ITERS);
    println(&format!("[smp] worker done acc={}", acc));
    sys_exit(0);
}

// ── Orchestrator-only helpers ─────────────────────────────────────────────────

fn spawn_worker() -> Option<usize> {
    sys_set_spawn_args("smp-worker");
    match sys_spawn_pinned(PROBE_PATH, TaskPriority::Normal as u8, 0) {
        SyscallResult::Ok(tid) => Some(tid),
        _ => None,
    }
}

/// Receive the exit notification for `tid` (caller must have already called
/// `sys_notify_on_exit(tid)`).  Loops past unrelated notifications.
fn recv_exit(tid: usize) {
    let mut buf = [0u8; 8];
    loop {
        if let SyscallResult::Ok(s) = sys_recv(0, &mut buf) {
            if s == tid { break; }
        }
    }
}

/// Register and await `tid`'s exit.
fn wait_exit(tid: usize) {
    let _ = sys_notify_on_exit(tid);
    recv_exit(tid);
}

// ── Scenario 1: spawn_rate ────────────────────────────────────────────────────

/// Sequential spawn throughput.  PASS iff ≥ 10 tasks/sec on single-hart QEMU TCG.
///
/// Tests that spawn+compute+exit overhead stays bounded. The 10/sec floor
/// accommodates single-hart QEMU TCG (observed ~15/sec); real 2-hart MTTCG
/// achieves ≥ 30/sec.
fn measure_spawn_rate() -> Option<bool> {
    const N: u64 = 8;
    const TARGET: u64 = 10; // ≥ 100 ms per full lifecycle budget (conservative for TCG)

    let t0 = sys_get_time();
    for _ in 0..N {
        match spawn_worker() {
            Some(tid) => wait_exit(tid),
            None => {
                println("[smp] spawn_rate SKIP — bench-probe not at /bin/bench-probe");
                return None;
            }
        }
    }
    let dt_ns = sys_get_time().saturating_sub(t0).saturating_mul(NS_PER_TICK);
    let per_sec = if dt_ns > 0 { N.saturating_mul(1_000_000_000) / dt_ns } else { u64::MAX };
    let pass = per_sec >= TARGET;
    println(&format!(
        "[smp] spawn_rate {}: {}/sec (target ≥{}/sec){}",
        if pass { "PASS" } else { "FAIL" }, per_sec, TARGET, CAVEAT
    ));
    Some(pass)
}

// ── Scenario 2: ipc_throughput ────────────────────────────────────────────────

/// Sustained IPC round-trips (orchestrator ↔ echo worker).
/// PASS iff ≥ 5 000 msgs/sec.
fn measure_ipc_throughput() -> Option<bool> {
    const MSGS: u64 = 1_000;
    const TARGET: u64 = 5_000;

    sys_set_spawn_args("ipc-echo");
    let echo_tid = match sys_spawn_pinned(PROBE_PATH, TaskPriority::Normal as u8, 0) {
        SyscallResult::Ok(t) => t,
        _ => {
            println("[smp] ipc_throughput SKIP — bench-probe not at /bin/bench-probe");
            return None;
        }
    };
    for _ in 0..20 { yield_now(); } // let echo reach its recv loop before measurement

    let mut rbuf = [0u8; 8];
    let t0 = sys_get_time();
    for _ in 0..MSGS {
        let _ = sys_send(echo_tid, &[1u8]);
        let _ = sys_recv(0, &mut rbuf);
    }
    let dt_ns = sys_get_time().saturating_sub(t0).saturating_mul(NS_PER_TICK);
    let _ = sys_force_exit(echo_tid);

    let per_sec = if dt_ns > 0 { MSGS.saturating_mul(1_000_000_000) / dt_ns } else { u64::MAX };
    let pass = per_sec >= TARGET;
    println(&format!(
        "[smp] ipc_throughput {}: {}/sec (target ≥{}/sec){}",
        if pass { "PASS" } else { "FAIL" }, per_sec, TARGET, CAVEAT
    ));
    Some(pass)
}

// ── Scenario 3: work_distribution ─────────────────────────────────────────────

/// 2-hart scale factor: PASS iff 2×T_single / T_parallel ≥ 1.40.
///
/// T_single: orchestrator idle while 1 worker runs.
/// T_parallel: orchestrator compute + 1 worker run concurrently on 2 harts.
/// On MTTCG harts both run on separate host threads → scale ≈ 2×.
fn measure_work_distribution() -> Option<bool> {
    // T_single: spawn + wait; orchestrator does nothing.
    let t0 = sys_get_time();
    let tid1 = spawn_worker().or_else(|| {
        println("[smp] work_distribution SKIP — bench-probe not at /bin/bench-probe");
        None
    })?;
    wait_exit(tid1);
    let t_single = sys_get_time().saturating_sub(t0);

    // T_parallel: spawn + register notify before orchestrator compute to prevent
    // a race where the worker exits before notify_on_exit is registered.
    let t1 = sys_get_time();
    let tid2 = spawn_worker().or_else(|| {
        println("[smp] work_distribution SKIP — bench-probe unavailable for T_parallel run");
        None
    })?;
    let _ = sys_notify_on_exit(tid2); // must register before compute loop
    let _ = compute(SMP_WORKER_ITERS); // orchestrator's contribution (hart A)
    recv_exit(tid2); // worker ran on hart B (or was stolen by it)
    let t_parallel = sys_get_time().saturating_sub(t1);

    // scale_x100 = 2 × T_single × 100 / T_parallel
    let scale_x100 = if t_parallel > 0 {
        t_single.saturating_mul(200) / t_parallel
    } else {
        200 // guard against zero denominator → report 2.00×
    };
    let pass = scale_x100 >= 140;
    println(&format!(
        "[smp] work_distribution {}: scale={}.{:02}x T1={}t Tp={}t (target ≥1.40x){}",
        if pass { "PASS" } else { "FAIL" },
        scale_x100 / 100, scale_x100 % 100,
        t_single, t_parallel, CAVEAT
    ));
    Some(pass)
}

// ── Suite entry ───────────────────────────────────────────────────────────────

/// Run all 3 SMP throughput scenarios; returns (passed, failed).
/// SKIP scenarios count as neither.
pub fn run_smp_suite() -> (u32, u32) {
    let mut passed = 0u32;
    let mut failed = 0u32;
    for result in [
        measure_spawn_rate(),
        measure_ipc_throughput(),
        measure_work_distribution(),
    ] {
        match result {
            Some(true)  => passed += 1,
            Some(false) => failed += 1,
            None        => {} // SKIP
        }
    }
    (passed, failed)
}
