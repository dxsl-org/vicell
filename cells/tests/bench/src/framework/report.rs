//! Benchmark report: statistics computation and JSON/text emission.

extern crate alloc;
use alloc::vec::Vec;
use api::benchmark::BenchReport;
use super::timer::ticks_to_ns;
use ostd::io::println;

/// Compute a `BenchReport` from a raw (unsorted) tick-delta sample buffer.
///
/// Sorts `samples` in-place, converts each tick delta to nanoseconds, then
/// builds percentile stats.
pub fn build_report(name: &'static str, samples: &mut Vec<u64>) -> BenchReport {
    samples.sort_unstable();
    let ns: Vec<u64> = samples.iter().map(|&t| ticks_to_ns(t)).collect();
    BenchReport::from_sorted(name, &ns)
}

/// Print a human-readable summary of a `BenchReport` to the serial console.
pub fn print_report(r: &BenchReport) {
    println(&format_report(r));
}

/// Print a machine-readable JSON line (for CI parsing).
pub fn print_json(r: &BenchReport) {
    let mut buf = [0u8; 256];
    let len = r.write_json(&mut buf);
    if len > 0 {
        if let Ok(s) = core::str::from_utf8(&buf[..len]) {
            println(s);
        }
    }
}

/// Format a single-line human-readable report string.
fn format_report(r: &BenchReport) -> alloc::string::String {
    use alloc::format;
    format!(
        "[bench] {:20} n={:>6}  min={:>6}ns  p50={:>6}ns  p99={:>6}ns  max={:>6}ns",
        r.name, r.n, r.min, r.p50, r.p99, r.max
    )
}
