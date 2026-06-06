#![no_std]
#![no_main]
// Note: #[no_mangle] on main() is required by the ViCell ELF loader and
// triggers unsafe_attr, so we cannot use #![forbid(unsafe_code)] here.
// All benchmark logic in framework/ and scenarios/ is unsafe-free.

extern crate alloc;

mod framework;
mod scenarios;

use api::benchmark::ViBenchmark;
use framework::{report, runner};
use ostd::io::println;
use scenarios::{
    context_switch::ContextSwitchBench,
    ipc_send_recv::IpcSendRecvBench,
    memory_footprint::MemoryFootprintBench,
    syscall_yield::SyscallYieldBench,
};

/// PDR performance targets (nanoseconds).  All checked against p99.
const TARGET_CTX_SWITCH_NS:  u64 = 100_000; //  100 µs
const TARGET_IPC_NS:         u64 =  50_000; //   50 µs
const TARGET_SYSCALL_NS:     u64 =  10_000; //   10 µs
const TARGET_FOOTPRINT_BYTES: u64 = 10 * 1024 * 1024; // 10 MB

/// Path this binary is spawned from (used to re-spawn itself in load/probe roles).
/// Populated when the bench cell is embedded into `/bin` (plan phase-05).
const SELF_PATH: &str = "/bin/bench";
/// Number of background load cells for RT-under-contention measurement.
const LOAD_CELLS: usize = 3;

/// Re-spawn this binary in `role` at the given priority, pinned to core 0.
/// Returns the new task id, or `None` if spawn failed (e.g. /bin/bench absent).
fn spawn_role(role: &str, priority: u8) -> Option<usize> {
    ostd::syscall::sys_set_spawn_args(role);
    match ostd::syscall::sys_spawn_pinned(SELF_PATH, priority, 0) {
        ostd::syscall::SyscallResult::Ok(tid) => Some(tid),
        _ => None,
    }
}

/// Orchestrate the RealTime preempt-latency scenario: spawn load + probe,
/// measure wake-to-run latency under contention, report, then clean up.
fn run_rt_preempt() {
    use api::task::TaskPriority;
    // Background load (Normal priority).
    let mut load_tids = [0usize; LOAD_CELLS];
    for slot in load_tids.iter_mut() {
        if let Some(tid) = spawn_role("load", TaskPriority::Normal as u8) {
            *slot = tid;
        }
    }
    // RealTime probe.
    let Some(probe_tid) = spawn_role("rt-probe", TaskPriority::RealTime as u8) else {
        println("[rt] preempt_latency SKIP — probe spawn failed (bench not at /bin/bench yet)");
        return;
    };
    // Let cells reach their loops before measuring.
    for _ in 0..100 { ostd::task::yield_now(); }

    let r = scenarios::preempt_latency::measure(probe_tid);
    r.print();
    r.print_json();
    // PDR-ish placeholder target (200 µs p99); first real run calibrates baseline.
    if r.meets(200_000) { println("[rt] preempt_latency PASS"); }
    else { println("[rt] preempt_latency FAIL (p99 over 200µs or deadline miss)"); }

    // Tear down spawned cells.
    let _ = ostd::syscall::sys_force_exit(probe_tid);
    for &tid in &load_tids {
        if tid != 0 { let _ = ostd::syscall::sys_force_exit(tid); }
    }
}

#[no_mangle]
pub fn main() {
    // Multi-role dispatch: load/rt-probe cells are re-spawns of this binary with
    // a role arg; the default (no arg) role is the orchestrator.
    let mut argbuf = [0u8; 32];
    let an = ostd::syscall::sys_spawn_args(&mut argbuf);
    match core::str::from_utf8(&argbuf[..an]).unwrap_or("") {
        "load" => scenarios::rt_load::run_load(),
        "rt-probe" => scenarios::preempt_latency::run_probe(),
        _ => {} // orchestrator falls through
    }

    println("[bench] ViCell Performance Benchmark Suite v0.1");
    println("[bench] PDR targets: ctx<100µs  ipc<50µs  syscall<10µs  mem<10MB");
    println("");

    let mut passed = 0u32;
    let mut failed = 0u32;

    // ── 1. Context switch ─────────────────────────────────────────────────────
    {
        let r = runner::run_default(&mut ContextSwitchBench);
        report::print_report(&r);
        report::print_json(&r);
        if r.meets_target(TARGET_CTX_SWITCH_NS) {
            passed += 1;
            println("[bench] context_switch PASS");
        } else {
            failed += 1;
            println("[bench] context_switch FAIL (p99 exceeds 100µs target)");
        }
    }

    // ── 2. IPC send/recv ──────────────────────────────────────────────────────
    {
        let r = runner::run_default(&mut IpcSendRecvBench::new());
        report::print_report(&r);
        report::print_json(&r);
        if r.meets_target(TARGET_IPC_NS) {
            passed += 1;
            println("[bench] ipc_send_recv PASS");
        } else {
            failed += 1;
            println("[bench] ipc_send_recv FAIL (p99 exceeds 50µs target)");
        }
    }

    // ── 3. Syscall yield ─────────────────────────────────────────────────────
    {
        let r = runner::run_default(&mut SyscallYieldBench);
        report::print_report(&r);
        report::print_json(&r);
        if r.meets_target(TARGET_SYSCALL_NS) {
            passed += 1;
            println("[bench] syscall_yield PASS");
        } else {
            failed += 1;
            println("[bench] syscall_yield FAIL (p99 exceeds 10µs target)");
        }
    }

    // ── 4. Memory footprint ───────────────────────────────────────────────────
    {
        let mut mb = MemoryFootprintBench::new();
        let _ = mb.run_once();
        let r = mb.footprint_report();
        report::print_report(&r);
        report::print_json(&r);
        if r.p50 <= TARGET_FOOTPRINT_BYTES {
            passed += 1;
            println("[bench] memory_footprint PASS");
        } else {
            failed += 1;
            println("[bench] memory_footprint FAIL (exceeds 10 MB target)");
        }
    }

    // ── 5. RT preempt latency (under load) ────────────────────────────────────
    println("");
    println("[rt] Real-time latency suite (under load):");
    run_rt_preempt();

    // ── Summary ───────────────────────────────────────────────────────────────
    println("");
    println(&alloc::format!(
        "[bench] Results: {}/{} PASS  {}/{} FAIL",
        passed, passed + failed, failed, passed + failed
    ));

    if failed == 0 {
        println("[bench] ALL BENCHMARKS PASS");
    } else {
        println("[bench] BENCHMARK FAILURES DETECTED");
    }
}
