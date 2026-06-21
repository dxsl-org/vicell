//! Context-switch latency benchmark.
//!
//! Measures the round-trip cost of `sys_yield` — a proxy for the scheduler's
//! context-switch overhead.  Two consecutive yields are timed; the delta
//! approximates one context switch.  PDR target: < 100 µs on QEMU.

use api::benchmark::ViBenchmark;
use ostd::syscall::sys_yield;

pub struct ContextSwitchBench;

impl ViBenchmark for ContextSwitchBench {
    fn name(&self) -> &'static str { "context_switch" }

    fn run_once(&mut self) -> api::ViResult<u64> {
        // Yield twice; the caller times the whole run_once() call, so the
        // measured delta ≈ one round-trip through the scheduler.
        sys_yield();
        sys_yield();
        Ok(0) // timing handled externally by runner
    }
}
