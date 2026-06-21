#![no_std]
#![no_main]
// Note: #[no_mangle] on main() is required by the ViCell ELF loader and
// triggers unsafe_attr, so we cannot use #![forbid(unsafe_code)] here.
// All benchmark logic in framework/ and scenarios/ is unsafe-free.

extern crate alloc;

mod framework;
mod scenarios;

api::declare_syscalls![Send, Recv, TryRecv, Log, Heartbeat, GetTime, SetTimer, SpawnPinned, StateStash, StateRestore, Exit, Yield];
api::declare_manifest!(block_io = false, network = false, spawn = true);

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
// QEMU TCG target (real hardware target is 10 µs; TCG adds 2-4× overhead).
const TARGET_SYSCALL_NS:     u64 =  40_000; //   40 µs (QEMU TCG; real-HW target: 10 µs)
const TARGET_FOOTPRINT_BYTES: u64 = 10 * 1024 * 1024; // 10 MB

/// Path for probe/load child cells.  A separate binary (bench-probe) is used
/// so both orchestrator and children can coexist in the SAS page table without
/// VA collision (bench @ 0x18000000, bench-probe @ 0x19000000).
const SELF_PATH: &str = "/bin/bench-probe";
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

/// Spawn `LOAD_CELLS` Normal-priority load cells; returns their tids (0 = failed).
fn spawn_load() -> [usize; LOAD_CELLS] {
    use api::task::TaskPriority;
    let mut tids = [0usize; LOAD_CELLS];
    for slot in tids.iter_mut() {
        if let Some(tid) = spawn_role("load", TaskPriority::Normal as u8) { *slot = tid; }
    }
    tids
}

/// Force-exit every non-zero tid in `tids`.
fn kill_all(tids: &[usize]) {
    for &tid in tids {
        if tid != 0 { let _ = ostd::syscall::sys_force_exit(tid); }
    }
}

/// Control-loop jitter scenario: a RealTime cell measures its own period
/// adherence under load and prints the report itself; we just orchestrate.
fn run_rt_control_loop() {
    use api::task::TaskPriority;
    let load_tids = spawn_load();
    let Some(probe_tid) = spawn_role("ctl-loop", TaskPriority::RealTime as u8) else {
        println("[rt] control_loop SKIP — probe spawn failed (bench not at /bin/bench yet)");
        kill_all(&load_tids);
        return;
    };
    for _ in 0..100 { ostd::task::yield_now(); }
    let _ = ostd::syscall::sys_send(probe_tid, &[0u8]);      // start ping
    let mut done = [0u8; 8];
    let _ = ostd::syscall::sys_recv(0, &mut done);            // wait for completion
    // The probe sys_exit's itself; only the load cells need teardown.
    kill_all(&load_tids);
}

/// IPC/syscall latency under load: idle baseline vs with load cells spinning.
fn run_rt_under_load() {
    let ipc_idle = runner::run_default(&mut IpcSendRecvBench::new());
    let sys_idle = runner::run_default(&mut SyscallYieldBench);

    let load_tids = spawn_load();
    if load_tids.iter().all(|&t| t == 0) {
        println("[rt] under_load SKIP — load spawn failed (bench not at /bin/bench yet)");
        return;
    }
    for _ in 0..100 { ostd::task::yield_now(); }

    let ipc_load = runner::run_default(&mut IpcSendRecvBench::new());
    let sys_load = runner::run_default(&mut SyscallYieldBench);
    print_under_load("ipc_send_recv", ipc_idle.p99, ipc_load.p99);
    print_under_load("syscall_yield", sys_idle.p99, sys_load.p99);
    kill_all(&load_tids);
}

/// Print idle vs under-load p99 plus the integer ratio (×100 → fixed-point x.xx).
fn print_under_load(name: &str, idle_p99: u64, load_p99: u64) {
    let ratio = if idle_p99 > 0 { load_p99.saturating_mul(100) / idle_p99 } else { 0 };
    println(&alloc::format!(
        "[rt] {:14} idle_p99={}ns load_p99={}ns ratio={}.{:02}x",
        name, idle_p99, load_p99, ratio / 100, ratio % 100
    ));
}

#[no_mangle]
pub fn main() {
    // Multi-role dispatch: load/rt-probe cells are re-spawns of this binary with
    // a role arg; the default (no arg) role is the orchestrator.
    let mut argbuf = [0u8; 32];
    let an = ostd::syscall::sys_spawn_args(&mut argbuf);
    match core::str::from_utf8(&argbuf[..an]).unwrap_or("") {
        "load"     => scenarios::rt_load::run_load(),
        "rt-probe" => scenarios::preempt_latency::run_probe(),
        "ctl-loop" => scenarios::control_loop::run_control_loop(),
        "ipc-echo" => {
            // Dedicated IPC echo peer: recv any message, reply to sender, repeat.
            let mut buf = [0u8; 64];
            loop {
                let sender = match ostd::syscall::sys_recv(0, &mut buf) {
                    ostd::syscall::SyscallResult::Ok(sid) => sid,
                    _ => continue,
                };
                ostd::syscall::sys_send(sender, &[]);
            }
        }
        "smp-worker" => scenarios::smp::run_worker(),
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
            println("[bench] syscall_yield FAIL (p99 exceeds 40µs QEMU target; real-HW target: 10µs)");
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
    run_rt_control_loop();
    run_rt_under_load();

    // ── 6. SMP throughput (work-stealing, 2 harts) ────────────────────────────
    println("");
    println("[smp] SMP throughput suite (2-hart work-stealing):");
    let (sp, sf) = scenarios::smp::run_smp_suite();
    passed += sp;
    failed += sf;

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
