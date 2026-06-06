//! preempt_latency — RealTime wake-to-run latency under load.
//!
//! The orchestrator sends a timestamped message to the RealTime probe; the probe
//! (spawned via `spawn_pinned` at RealTime priority) wakes, reads the clock, and
//! the orchestrator records `t1 − t0`. Sampled with N background load cells
//! spinning, the delta approximates the scheduler's preempt + IPC wake latency
//! for a RealTime cell — the core RT-determinism figure.
//!
//! ⚠️ Runtime verification needs the bench cell embedded at `/bin/bench`
//! (plan phase-05). Source compiles + type-checks standalone; numbers require a boot.

extern crate alloc;
use alloc::vec::Vec;
use crate::framework::rt_report::RtReport;
use ostd::syscall::{sys_get_time, sys_recv, sys_send, SyscallResult};

const WARMUP: u32 = 50;
const ITERS: u32 = 500;

/// Probe role (RealTime): block on recv, stamp arrival, reply with the delta.
///
/// Calls a syscall (recv + send) every iteration so the never-die RT watchdog
/// — which is RealTime-only and resets on syscall — never trips. Never returns.
pub fn run_probe() -> ! {
    let mut buf = [0u8; 16];
    loop {
        buf.fill(0);
        match sys_recv(0, &mut buf) {
            SyscallResult::Ok(sender) if sender > 0 => {
                let t1 = sys_get_time();
                let t0 = u64::from_le_bytes(buf[..8].try_into().unwrap_or([0; 8]));
                let delta = t1.saturating_sub(t0);
                sys_send(sender, &delta.to_le_bytes());
            }
            _ => ostd::task::yield_now(),
        }
    }
}

/// Orchestrator side: drive the probe `WARMUP + ITERS` times and report deltas.
pub fn measure(probe_tid: usize) -> RtReport {
    let mut samples: Vec<u64> = Vec::with_capacity(ITERS as usize);
    let mut rbuf = [0u8; 8];
    for i in 0..(WARMUP + ITERS) {
        let t0 = sys_get_time();
        let _ = sys_send(probe_tid, &t0.to_le_bytes());
        let _ = sys_recv(0, &mut rbuf);
        if i >= WARMUP {
            samples.push(u64::from_le_bytes(rbuf));
        }
    }
    RtReport::build("preempt_latency", &mut samples, 0)
}
