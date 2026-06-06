#![no_std]
#![no_main]

extern crate ostd;

// Declares spawn capability; the kernel grants SpawnCap at spawn.
api::declare_manifest!(block_io = false, network = false, spawn = true);

use ostd::io::println;

/// Kernel spawns init from its embedded ELF.  Init's job is to bring up the
/// rest of the system by loading cell ELFs from the bootstrap disk table.
///
/// Boot sequence:
///   1. Spawn VFS service — serves `/bin/*` once running.
///   2. Spawn Config service — configuration KV store.
///   3. Spawn Shell — interactive REPL.
#[no_mangle]
pub extern "C" fn main() {
    use ostd::syscall::{sys_notify_on_exit, sys_recv, sys_spawn_from_path, SyscallResult};
    println("Init: Starting ViCell Orchestrator...");

    // Supervised services in bring-up order — VFS first (it serves /bin/*).
    // tids[i] is the current live tid of paths[i] (None when down).
    const NSVC: usize = 6;
    let paths: [&str; NSVC] = [
        "/bin/vfs",
        "/bin/config",
        "/bin/input",
        "/bin/net",
        "/bin/compositor",
        "/bin/shell",
    ];
    let mut tids: [Option<usize>; NSVC] = [None; NSVC];

    for i in 0..NSVC {
        match sys_spawn_from_path(paths[i]) {
            SyscallResult::Ok(tid) => tids[i] = Some(tid),
            // Non-fatal: input/net/compositor may be absent (no device/binary).
            _ => {}
        }
        // Let each service initialise before the next; VFS gets an extra beat to
        // register /bin/* before the others try to load from it.
        ostd::task::yield_now();
        if i == 0 {
            ostd::task::yield_now();
        }
    }
    println("Init: services spawned.");

    // Optional benchmark suite (CI disk images only) — not supervised.
    let _ = sys_spawn_from_path("/bin/bench");

    // Register a death notification for every live service. A single recv loop
    // below now supervises ALL of them (wait-any): when any service exits or
    // faults, sys_recv returns its tid and we respawn it. This is the full
    // supervisor tree built on NotifyOnExit (Law 1 syscall 204).
    for t in tids.iter().flatten() {
        let _ = sys_notify_on_exit(*t);
    }
    println("Init: supervising services (auto-restart on crash)...");

    // Death notifications carry the dead tid as the recv "sender"; no payload, so a
    // tiny throwaway buffer suffices.
    let mut buf = [0u8; 16];
    let mut restarts: u32 = 0;
    const MAX_RESTARTS: u32 = 200;
    loop {
        let dead = match sys_recv(0, &mut buf) {
            SyscallResult::Ok(d) => d,
            _ => {
                ostd::task::yield_now();
                continue;
            }
        };
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
        if restarts >= MAX_RESTARTS {
            println("Init: restart limit reached — backing off supervision.");
            tids[i] = None;
            continue;
        }
        restarts += 1;
        println("Init: service died — restarting...");
        match sys_spawn_from_path(paths[i]) {
            SyscallResult::Ok(newt) => {
                tids[i] = Some(newt);
                let _ = sys_notify_on_exit(newt); // re-arm for the new instance
                println("Init: service restarted.");
            }
            _ => {
                tids[i] = None;
                println("Init: service restart FAILED.");
            }
        }
    }
}
