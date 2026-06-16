#![no_std]
#![no_main]

extern crate ostd;

// Declares spawn capability; the kernel grants SpawnCap at spawn.
api::declare_manifest!(block_io = false, network = false, spawn = true);

// Narrow syscall allowlist — kernel enforces this at dispatch (Phase 27).
// ForceExit, NotifyOnExit, RegisterService are always-permitted (SpawnCap-gated).
api::declare_syscalls![
    Send, Recv, TryRecv, RecvTimeout, Reply, Log, Heartbeat, LookupService,
    SpawnFromPath, SpawnFromMem, SpawnPinned, Wait, GetTime, SetTimer,
    HotSwap, StateStash, StateRestore,
];

use ostd::io::println;

/// Per-service restart policy (OTP-style).
#[derive(Clone, Copy, PartialEq)]
enum Policy {
    /// Always restart — critical services that must always be up (vfs, shell, …).
    Permanent,
    /// Restart only on ABNORMAL exit (fault / watchdog kill); a clean exit (reason 0)
    /// is treated as final. Uses the exit reason delivered as the death-notify payload.
    Transient,
    /// Never restart — one-shot tasks that are expected to run once and stop.
    Temporary,
}

/// Restart intensity: at most this many restarts of ONE service within
/// `RESTART_WINDOW_TICKS`. Exceeding it is a crash storm → escalate (give up on that
/// service) instead of spin-respawning forever, which would burn CPU and never recover.
/// Ticks are 10 ms scheduler ticks, so 1000 ≈ 10 s.
const MAX_RESTARTS_PER_WINDOW: u32 = 5;
const RESTART_WINDOW_TICKS: u64 = 1000;

/// Kernel spawns init from its embedded ELF.  Init's job is to bring up the
/// rest of the system by loading cell ELFs from the bootstrap disk table.
///
/// Boot sequence:
///   1. Spawn VFS service — serves `/bin/*` once running.
///   2. Spawn Config service — configuration KV store.
///   3. Spawn Shell — interactive REPL.
#[no_mangle]
pub extern "C" fn main() {
    use ostd::syscall::{
        sys_get_time, sys_lookup_service, sys_notify_on_exit, sys_recv, sys_register_service,
        sys_spawn_from_path, SyscallResult,
    };
    use api::syscall::service;
    println("Init: Starting ViCell Orchestrator...");

    // Supervised services in bring-up order — VFS first (it serves /bin/*).
    // tids[i] is the current live tid of paths[i] (None when down).
    const NSVC: usize = 9;
    let paths: [&str; NSVC] = [
        "/bin/vfs",
        "/bin/config",
        "/bin/input",
        "/bin/net",
        "/bin/compositor",
        "/bin/silo",             // Security Silo — P-256 key isolation (Tier 3a)
        "/bin/shell",
        "/bin/robot-demo",       // G1 sensor→actuator→MQTT reference demo
        "/bin/robot-dashboard",  // G1 ViUI v2 dashboard demo (FramebufferRenderer)
    ];
    let mut tids: [Option<usize>; NSVC] = [None; NSVC];

    // Well-known service ID per path (None = not a looked-up service, e.g. shell).
    // The supervisor registers each service's CURRENT tid here so clients resolve it
    // via sys_lookup_service and reconnect transparently across a respawn.
    let svc_ids: [Option<u16>; NSVC] = [
        Some(service::VFS),
        Some(service::CONFIG),
        Some(service::INPUT),
        Some(service::NET),
        Some(service::COMPOSITOR),
        Some(types::silo::SILO_SERVICE_ID), // SILO = 6
        None, // shell is not a registered service
        None, // robot-demo is not a registered service
        None, // robot-dashboard is not a registered service
    ];

    // Restart policy per service. All current services are Permanent (a robot must keep
    // them up); the machinery supports Transient (restart only on abnormal exit) and
    // Temporary (never restart) for future one-shot/optional cells.
    let policy: [Policy; NSVC] = [
        Policy::Permanent, // vfs
        Policy::Permanent, // config
        Policy::Permanent, // input
        Policy::Permanent, // net
        Policy::Permanent, // compositor
        Policy::Permanent, // silo — key service, must stay up
        Policy::Transient, // shell: restart on crash, but a clean `exit` is final
        Policy::Temporary, // robot-demo: run once then stop
        Policy::Temporary, // robot-dashboard: ViUI demo, run once
    ];
    // Per-service restart-intensity state (sliding window).
    let mut restart_count: [u32; NSVC] = [0; NSVC];
    let mut window_start: [u64; NSVC] = [0; NSVC];

    for i in 0..NSVC {
        match sys_spawn_from_path(paths[i]) {
            SyscallResult::Ok(tid) => {
                tids[i] = Some(tid);
                if let Some(sid) = svc_ids[i] {
                    let _ = sys_register_service(sid, tid);
                }
            }
            // Non-fatal: input/net/compositor may be absent (no device/binary on VF2).
            _ => {
                println("Init: cell not found — skipping:");
                println(paths[i]);
            }
        }
        // Let each service initialise before the next; VFS gets an extra beat to
        // register /bin/* before the others try to load from it.
        ostd::task::yield_now();
        if i == 0 {
            ostd::task::yield_now();
        }
    }
    println("Init: services spawned.");

    // Service-registry round-trip self-check (observable boot proof): every registered
    // service must resolve via sys_lookup_service to the tid we recorded at spawn.
    let mut ok = true;
    for i in 0..NSVC {
        if let (Some(sid), Some(tid)) = (svc_ids[i], tids[i]) {
            if sys_lookup_service(sid) != Some(tid) {
                ok = false;
            }
        }
    }
    if ok {
        println("Init: service registry verified.");
    } else {
        println("Init: WARN service registry mismatch.");
    }

    // Optional benchmark suite (CI disk images only) — not supervised.
    let _ = sys_spawn_from_path("/bin/bench");
    // Optional peripheral demo (GPIO/UART) — AArch64 only, no-op on RISC-V.
    let _ = sys_spawn_from_path("/bin/periph-demo");
    // Optional I2C sensor demo (SHT3x, bit-bang over GPIO pins 0/1) — AArch64 only.
    let _ = sys_spawn_from_path("/bin/sensor-demo");
    // Optional SPI demo (bit-bang over GPIO pins 2/3/4/5) — AArch64 only.
    let _ = sys_spawn_from_path("/bin/spi-demo");
    // Optional PWM bit-bang demo (50 Hz servo sweep over GPIO) — AArch64 only.
    let _ = sys_spawn_from_path("/bin/pwm-demo");
    // Optional ADC simulation demo (triangle-wave ramp channels) — all arches.
    let _ = sys_spawn_from_path("/bin/adc-demo");
    // Optional CAN loopback demo (in-memory loopback, no hardware) — all arches.
    let _ = sys_spawn_from_path("/bin/can-demo");
    // Optional hypervisor service — auto-starts when /bin/hypervisor is present in
    // the disk image (aarch64 + virtualization=on kernel builds only).
    let _ = sys_spawn_from_path("/bin/hypervisor");
    // Optional silo integration test — only present in test disk images.
    let _ = sys_spawn_from_path("/bin/silo-test");
    // VFS integration test suite: only present in test-hooks kernel images.
    let _ = sys_spawn_from_path("/bin/vfs-test");
    // Bare-cell input delivery test — ostd::input focus + event loop, no viui.
    // Only present in test disk images; silently skipped in production.
    let _ = sys_spawn_from_path("/bin/input-test");

    // Register a death notification for every live service. A single recv loop
    // below now supervises ALL of them (wait-any): when any service exits or
    // faults, sys_recv returns its tid and we respawn it. This is the full
    // supervisor tree built on NotifyOnExit (Law 1 syscall 204).
    for t in tids.iter().flatten() {
        let _ = sys_notify_on_exit(*t);
    }
    println("Init: supervising services (auto-restart on crash)...");

    // Death notifications: sys_recv returns the dead tid; the kernel writes the exit
    // reason (NotifyOnExit payload) into the first 8 bytes of buf (0 = clean exit,
    // usize::MAX = fault / watchdog kill). The policy below uses it.
    let mut buf = [0u8; 16];
    loop {
        let dead = match sys_recv(0, &mut buf) {
            SyscallResult::Ok(d) => d,
            _ => {
                ostd::task::yield_now();
                continue;
            }
        };
        let reason = u64::from_le_bytes([
            buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
        ]);
        // Which supervised service died? (Ignore notifications for unknown tids.)
        let mut which = None;
        for (i, t) in tids.iter().enumerate() {
            if *t == Some(dead) {
                which = Some(i);
                break;
            }
        }
        let i = match which {
            Some(i) => i,
            None => continue,
        };

        // 1. Restart policy: decide whether this exit warrants a restart at all.
        let should_restart = match policy[i] {
            Policy::Temporary => false,
            Policy::Transient => reason != 0, // restart only on abnormal exit
            Policy::Permanent => true,
        };
        if !should_restart {
            println("Init: service exited cleanly — policy says no restart.");
            tids[i] = None;
            continue;
        }

        // 2. Restart intensity: bound restarts per sliding window; a crash storm escalates
        //    (give up on this one service) instead of spin-respawning forever.
        let now = sys_get_time();
        if now.wrapping_sub(window_start[i]) > RESTART_WINDOW_TICKS {
            window_start[i] = now;
            restart_count[i] = 0;
        }
        if restart_count[i] >= MAX_RESTARTS_PER_WINDOW {
            println("Init: restart storm — giving up on this service (escalate).");
            tids[i] = None;
            continue;
        }
        restart_count[i] += 1;

        println("Init: service died — restarting...");
        match sys_spawn_from_path(paths[i]) {
            SyscallResult::Ok(newt) => {
                tids[i] = Some(newt);
                let _ = sys_notify_on_exit(newt); // re-arm for the new instance
                if let Some(sid) = svc_ids[i] {
                    // Re-point the service registry at the new instance so clients that
                    // resolve via sys_lookup_service reconnect to the restarted service.
                    let _ = sys_register_service(sid, newt);
                }
                println("Init: service restarted.");
            }
            _ => {
                tids[i] = None;
                println("Init: service restart FAILED.");
            }
        }
    }
}
