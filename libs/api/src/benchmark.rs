// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Benchmarking framework for performance validation.
//!
//! Provides interfaces for measuring and validating performance
//! of critical operations in ViOS.

use crate::*;
use alloc::vec::Vec;
use alloc::boxed::Box;

/// Benchmark result with timing and metadata.
#[derive(Debug, Clone)]
pub struct BenchmarkResult {
    /// Benchmark name
    pub name: &'static str,
    /// Number of iterations
    pub iterations: u64,
    /// Total cycles elapsed
    pub total_cycles: u64,
    /// Average cycles per iteration
    pub avg_cycles: u64,
    /// Minimum cycles observed
    pub min_cycles: u64,
    /// Maximum cycles observed
    pub max_cycles: u64,
    /// Standard deviation
    pub std_dev: u64,
}

impl BenchmarkResult {
    /// Check if benchmark meets performance target.
    pub fn meets_target(&self, target_cycles: u64) -> bool {
        self.avg_cycles <= target_cycles
    }
}

/// Benchmark trait for performance tests.
pub trait ViBenchmark {
    /// Get benchmark name.
    fn name(&self) -> &'static str;

    /// Setup before benchmark run.
    fn setup(&mut self) -> ViResult<()> {
        Ok(())
    }

    /// Run one iteration of the benchmark.
    ///
    /// # Returns
    /// Cycles elapsed for this iteration.
    fn run_once(&mut self) -> ViResult<u64>;

    /// Teardown after benchmark run.
    fn teardown(&mut self) -> ViResult<()> {
        Ok(())
    }

    /// Run benchmark with specified iterations.
    fn run(&mut self, iterations: u64) -> ViResult<BenchmarkResult> {
        self.setup()?;

        let mut total = 0u64;
        let mut min = u64::MAX;
        let mut max = 0u64;
        let mut samples = [0u64; 100]; // For std dev calculation

        for i in 0..iterations {
            let cycles = self.run_once()?;
            total += cycles;
            min = min.min(cycles);
            max = max.max(cycles);
            
            // Store sample for std dev (up to 100 samples)
            if i < 100 {
                samples[i as usize] = cycles;
            }
        }

        self.teardown()?;

        let avg = total / iterations;
        let std_dev = calculate_std_dev(&samples[..iterations.min(100) as usize], avg);

        Ok(BenchmarkResult {
            name: self.name(),
            iterations,
            total_cycles: total,
            avg_cycles: avg,
            min_cycles: min,
            max_cycles: max,
            std_dev,
        })
    }
}

/// Calculate standard deviation.
fn calculate_std_dev(samples: &[u64], mean: u64) -> u64 {
    if samples.is_empty() {
        return 0;
    }

    let variance: u64 = samples.iter()
        .map(|&x| {
            let diff = if x > mean { x - mean } else { mean - x };
            diff * diff
        })
        .sum::<u64>() / samples.len() as u64;

    // Integer square root approximation
    integer_sqrt(variance)
}

/// Integer square root.
fn integer_sqrt(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// Benchmark suite for organizing multiple benchmarks.
pub trait ViBenchmarkSuite {
    /// Get suite name.
    fn name(&self) -> &'static str;

    /// Get all benchmarks in this suite.
    fn benchmarks(&mut self) -> &mut [Box<dyn ViBenchmark>];

    /// Run all benchmarks in suite.
    fn run_all(&mut self, iterations: u64) -> ViResult<Vec<BenchmarkResult>> {
        let mut results = Vec::new();
        // Force type hint if needed, though usually inferred
        for bench in self.benchmarks() {
            let res: BenchmarkResult = bench.run(iterations)?;
            results.push(res);
        }
        Ok(results)
    }
}

/// Performance targets for critical operations.
pub struct PerformanceTargets {
    /// File read (4KB) - target cycles
    pub file_read_4kb: u64,
    /// Network send (1KB) - target cycles
    pub net_send_1kb: u64,
    /// Hot-swap (1KB state) - target cycles
    pub hotswap_1kb: u64,
    /// VM-exit handling - target cycles
    pub vm_exit: u64,
    /// IPC roundtrip - target cycles
    pub ipc_roundtrip: u64,
}

impl Default for PerformanceTargets {
    fn default() -> Self {
        Self {
            file_read_4kb: 10_000,      // 10K cycles
            net_send_1kb: 5_000,         // 5K cycles
            hotswap_1kb: 50_000,         // 50K cycles
            vm_exit: 1_000,              // 1K cycles
            ipc_roundtrip: 2_000,        // 2K cycles
        }
    }
}
