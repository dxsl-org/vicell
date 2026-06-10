"""Write Phase 11 test infrastructure files."""
import os

base = "d:/ViCell"

# ── integration test harness ───────────────────────────────────────────────────
HARNESS = '''//! ViCell integration test harness.
//!
//! Provides `QemuRunner` which spawns QEMU, injects serial input, and
//! captures serial output.  Integration tests use this to drive the kernel.

use std::process::{Command, Child, Stdio};
use std::io::{BufReader, BufRead};
use std::time::{Duration, Instant};

/// QEMU-driven ViCell integration test runner.
pub struct QemuRunner {
    child: Child,
    output: Vec<String>,
}

impl QemuRunner {
    /// Spawn QEMU with `kernel` and begin capturing serial output on stdout.
    pub fn new_rv64(kernel: &str) -> Self {
        let child = Command::new("qemu-system-riscv64")
            .args(["-machine", "virt", "-nographic", "-bios", "default", "-kernel", kernel])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("qemu-system-riscv64 must be in PATH");
        Self { child, output: Vec::new() }
    }

    /// Block until `pattern` appears in serial output or `timeout_secs` elapses.
    ///
    /// Returns `Ok(line)` containing the matching line, or `Err` on timeout.
    pub fn wait_for(&mut self, pattern: &str, timeout_secs: u64) -> Result<String, String> {
        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        let stdout = self.child.stdout.take().expect("stdout must be piped");
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if Instant::now() > deadline {
                return Err(format!("timeout: pattern {:?} not seen in {}s", pattern, timeout_secs));
            }
            if let Ok(l) = line {
                self.output.push(l.clone());
                if l.contains(pattern) {
                    return Ok(l);
                }
            }
        }
        Err(format!("EOF before pattern {:?}", pattern))
    }

    /// Return true if any captured line contains `needle`.
    pub fn output_contains(&self, needle: &str) -> bool {
        self.output.iter().any(|l| l.contains(needle))
    }
}

impl Drop for QemuRunner {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}
'''

# ── scripts/measure-coverage.sh ───────────────────────────────────────────────
COVERAGE_SH = '''#!/usr/bin/env bash
# Measure line coverage for the ViCell kernel and libs using cargo llvm-cov.
# Requires: llvm-tools-preview component (pinned in rust-toolchain.toml).
# Usage: bash scripts/measure-coverage.sh
set -euo pipefail

cargo llvm-cov \\
  --workspace \\
  --target riscv64gc-unknown-none-elf \\
  -Z build-std=core,alloc \\
  --lcov \\
  --output-path lcov.info \\
  2>&1 | tee coverage.log

echo "Coverage report written to lcov.info"
echo "Open with: genhtml lcov.info -o coverage/"
'''

os.makedirs(f"{base}/tests/integration", exist_ok=True)

files = {
    f"{base}/tests/integration/harness.rs": HARNESS,
    f"{base}/scripts/measure-coverage.sh": COVERAGE_SH,
}

for path, content in files.items():
    if not os.path.exists(path):
        with open(path, "w", encoding="utf-8", newline="\n") as f:
            f.write(content)
        print(f"  wrote {path}")
    else:
        print(f"  exists {path}")

print("Done.")
