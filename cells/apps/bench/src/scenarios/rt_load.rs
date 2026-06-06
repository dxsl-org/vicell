//! Background load cell ("noisy neighbor") for RT scenarios.
//!
//! Spawned at Normal priority by the orchestrator to create scheduling
//! contention while the RealTime probe is measured. Busy-spins with an
//! occasional `yield` so Normal-priority cells still make progress — what we
//! measure is the RealTime probe *preempting* this load, not it being starved.
//!
//! Runs forever; the orchestrator force-exits the cell when the run completes.

use ostd::syscall::sys_yield;

/// Run the load loop forever. Never returns.
pub fn run_load() -> ! {
    loop {
        // Non-trivial arithmetic the optimizer cannot elide (black_box barrier).
        let mut acc = 0u64;
        for i in 0..50_000u64 {
            acc = acc.wrapping_add(i.wrapping_mul(3));
        }
        core::hint::black_box(acc);
        sys_yield();
    }
}
