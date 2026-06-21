//! Syscall overhead benchmark — raw yield syscall latency.
//!
//! Issues a single `Yield` syscall per iteration and measures the
//! round-trip from the ecall instruction back to the next user-mode
//! instruction.  PDR target: < 10 µs.

use api::benchmark::ViBenchmark;
use ostd::syscall::sys_yield;

pub struct SyscallYieldBench;

impl ViBenchmark for SyscallYieldBench {
    fn name(&self) -> &'static str { "syscall_yield" }

    fn run_once(&mut self) -> api::ViResult<u64> {
        sys_yield();
        Ok(0)
    }
}
