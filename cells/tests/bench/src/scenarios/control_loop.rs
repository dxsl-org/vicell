//! control_loop_jitter — periodic deadline adherence under load.
//!
//! A RealTime cell wakes every `PERIOD_TICKS` (via `recv_timeout`, which blocks
//! until timeout since nothing sends to it), measures the actual elapsed period,
//! and records the per-cycle error (|actual − period|) plus a deadline-miss when
//! the cycle overruns by more than `SLACK_TICKS`. Mirrors a fixed-rate control
//! loop (PID / software PWM) and proves "control-loop meets deadline" (G1 #3).
//!
//! Period is 50 ms (5 scheduler ticks) so jitter is meaningful above the 10 ms
//! tick quantum. Runs under background load cells spawned by the orchestrator.
//!
//! ⚠️ Runtime numbers require the bench cell embedded at `/bin/bench` (phase-05).

extern crate alloc;
use alloc::vec::Vec;
use crate::framework::rt_report::RtReport;
use ostd::syscall::{sys_get_time, sys_recv, sys_recv_timeout, sys_send, sys_exit, SyscallResult};

/// Target loop period: 50 ms @ 10 MHz mtime (5 scheduler ticks).
const PERIOD_TICKS: u64 = 50 * 10_000;
/// Allowed overrun before a cycle counts as a deadline miss: 5 ms.
const SLACK_TICKS: u64 = 5 * 10_000;
/// Number of measured periods.
const CL_ITERS: u32 = 200;

/// RealTime probe role: learn the orchestrator's tid from its start ping, run
/// the periodic loop, print the jitter/deadline report, then signal done + exit.
pub fn run_control_loop() -> ! {
    let mut buf = [0u8; 8];
    // Block for the orchestrator's start ping so we can reply "done" to it later.
    let orch = loop {
        match sys_recv(0, &mut buf) {
            SyscallResult::Ok(s) if s > 0 => break s,
            _ => ostd::task::yield_now(),
        }
    };

    let mut errors: Vec<u64> = Vec::with_capacity(CL_ITERS as usize);
    let mut miss = 0u32;
    let mut prev = sys_get_time();
    for _ in 0..CL_ITERS {
        // Sleep ~one period: recv_timeout returns on timeout (no sender) and
        // is a real block — the scheduler is free to run load cells meanwhile.
        let _ = sys_recv_timeout(0, &mut buf, PERIOD_TICKS);
        let now = sys_get_time();
        let actual = now.saturating_sub(prev);
        let err = if actual > PERIOD_TICKS { actual - PERIOD_TICKS } else { PERIOD_TICKS - actual };
        errors.push(err);
        if actual > PERIOD_TICKS + SLACK_TICKS { miss += 1; }
        prev = now;
    }

    let r = RtReport::build("control_loop_jitter", &mut errors, miss);
    r.print();
    r.print_json();
    if r.deadline_miss == 0 { ostd::io::println("[rt] control_loop PASS"); }
    else { ostd::io::println("[rt] control_loop FAIL (deadline misses)"); }

    let _ = sys_send(orch, &[1u8]); // signal orchestrator we are done
    sys_exit(0);
}
