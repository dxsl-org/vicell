//! Benchmark runner: warmup loop + measurement loop + percentile computation.

extern crate alloc;
use alloc::vec::Vec;
use api::benchmark::{BenchReport, ViBenchmark};
use super::{report, timer};

/// Default warmup iterations (discarded; exist to heat up caches and QEMU JIT).
pub const DEFAULT_WARMUP: u32 = 100;
/// Default measurement iterations per scenario.
pub const DEFAULT_ITERS: u32 = 1_000;

/// Run a benchmark through warmup + measurement and return a `BenchReport`.
///
/// # Arguments
/// * `bench` — mutable reference to the scenario implementing `ViBenchmark`
/// * `warmup` — number of iterations whose results are discarded
/// * `iters` — number of iterations to measure
pub fn run<B: ViBenchmark>(bench: &mut B, warmup: u32, iters: u32) -> BenchReport {
    // ── Warmup ────────────────────────────────────────────────────────────────
    bench.setup().ok();
    for _ in 0..warmup {
        let _ = bench.run_once();
    }

    // ── Measurement ──────────────────────────────────────────────────────────
    let mut samples: Vec<u64> = Vec::with_capacity(iters as usize);
    for _ in 0..iters {
        let t0 = timer::read_ticks();
        let _ = bench.run_once();
        let t1 = timer::read_ticks();
        samples.push(t1.saturating_sub(t0));
    }

    bench.teardown().ok();
    report::build_report(bench.name(), &mut samples)
}

/// Run a benchmark with the default warmup and iteration counts.
pub fn run_default<B: ViBenchmark>(bench: &mut B) -> BenchReport {
    run(bench, DEFAULT_WARMUP, DEFAULT_ITERS)
}
