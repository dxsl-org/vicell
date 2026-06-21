//! Real-time benchmark report — tail latency + jitter for RT determinism.
//!
//! `BenchReport` (libs/api) carries min/p50/p99/max. RT determinism cares about
//! the *tail* — p99.9, max, and jitter (max − min) — plus deadline-miss counts
//! for periodic loops. These are computed here in the bench cell so the stable
//! `BenchReport` ABI (Law 1) stays untouched.

extern crate alloc;
use alloc::vec::Vec;
use super::timer::ticks_to_ns;
use ostd::io::println;

/// RT latency statistics (nanoseconds), including jitter and deadline misses.
pub struct RtReport {
    pub name: &'static str,
    pub n: u32,
    pub min: u64,
    pub p50: u64,
    pub p99: u64,
    pub p99_9: u64,
    pub max: u64,
    /// Peak-to-peak spread (max − min) — the headline RT jitter figure.
    pub jitter: u64,
    /// Count of iterations that overran their deadline (periodic scenarios; 0 otherwise).
    pub deadline_miss: u32,
}

impl RtReport {
    /// Build from a raw (unsorted) tick-delta buffer; `deadline_miss` is supplied
    /// by periodic scenarios (else 0). Sorts in place and converts ticks → ns.
    pub fn build(name: &'static str, samples: &mut Vec<u64>, deadline_miss: u32) -> Self {
        samples.sort_unstable();
        let ns: Vec<u64> = samples.iter().map(|&t| ticks_to_ns(t)).collect();
        let n = ns.len();
        if n == 0 {
            return Self { name, n: 0, min: 0, p50: 0, p99: 0, p99_9: 0, max: 0, jitter: 0, deadline_miss };
        }
        // Percentile index, clamped to the last element for small sample counts.
        let pct = |num: usize, den: usize| -> u64 { ns[((n * num) / den).min(n - 1)] };
        let min = ns[0];
        let max = ns[n - 1];
        Self {
            name,
            n: n as u32,
            min,
            p50: pct(50, 100),
            p99: pct(99, 100),
            p99_9: pct(999, 1000),
            max,
            jitter: max - min,
            deadline_miss,
        }
    }

    /// Human-readable single line.
    pub fn print(&self) {
        use alloc::format;
        println(&format!(
            "[rt] {:18} n={:>5} min={:>7}ns p50={:>7}ns p99={:>7}ns p99.9={:>7}ns max={:>7}ns jitter={:>7}ns miss={}",
            self.name, self.n, self.min, self.p50, self.p99, self.p99_9, self.max, self.jitter, self.deadline_miss
        ));
    }

    /// Machine-readable JSON line for CI parsing (perf.yml / compare-bench-results.sh).
    pub fn print_json(&self) {
        use alloc::format;
        println(&format!(
            "{{\"name\":\"{}\",\"n\":{},\"min\":{},\"p50\":{},\"p99\":{},\"p999\":{},\"max\":{},\"jitter\":{},\"miss\":{}}}",
            self.name, self.n, self.min, self.p50, self.p99, self.p99_9, self.max, self.jitter, self.deadline_miss
        ));
    }

    /// PASS = p99 within `target_ns` AND zero deadline misses.
    pub fn meets(&self, target_ns: u64) -> bool {
        self.p99 <= target_ns && self.deadline_miss == 0
    }
}
