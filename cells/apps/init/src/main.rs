#![no_std]
#![no_main]

extern crate ostd;

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
    println("Init: Starting ViOS Orchestrator...");

    // 1. Spawn VFS Service (reads disk, serves /bin/*)
    println("Init: Spawning VFS Service...");
    match ostd::syscall::sys_spawn_from_path("/bin/vfs") {
        ostd::syscall::SyscallResult::Ok(_) => println("Init: VFS Service spawned."),
        _ => println("Init: WARN — VFS spawn failed; subsequent SpawnFromPath calls will fail."),
    }

    // Brief yield so VFS can initialise and register files before we ask for more.
    ostd::task::yield_now();
    ostd::task::yield_now();

    // 2. Spawn Config Service
    println("Init: Spawning Config Service...");
    match ostd::syscall::sys_spawn_from_path("/bin/config") {
        ostd::syscall::SyscallResult::Ok(_) => println("Init: Config Service spawned."),
        _ => println("Init: WARN — Config spawn failed."),
    }

    ostd::task::yield_now();

    // 3. Spawn Shell
    println("Init: Spawning Shell...");
    match ostd::syscall::sys_spawn_from_path("/bin/shell") {
        ostd::syscall::SyscallResult::Ok(_) => println("Init: Shell spawned successfully."),
        _ => println("Init: WARN — Shell spawn failed."),
    }

    // Keep init alive as the process supervisor.
    loop {
        ostd::task::yield_now();
    }
}
