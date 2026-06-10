#![no_std]
#![no_main]

extern crate alloc;
extern crate ostd;

// Declares spawn capability; the kernel grants SpawnCap at spawn.
api::declare_manifest!(block_io = false, network = false, spawn = true);

// Narrow syscall allowlist — kernel enforces this at dispatch (Phase 27).
// ForceExit is always-permitted (SpawnCap-gated at dispatch).
api::declare_syscalls![
    Send, Recv, TryRecv, RecvTimeout, Reply, Log, Heartbeat, LookupService,
    SpawnFromPath, SpawnFromMem, SpawnPinned, Wait, GetTime, GetProcs,
    HotSwap, StateStash, StateRestore,
    OpenCap, ReadCap, CloseCap,
    GrantAlloc, GrantShare, GrantSlice, GrantFree,
    // Read = stdin readline; Open/Close (+Read) = `cat` over the kernel FS;
    // Snapshot = the `snapshot` built-in. Omitting Read silently bricked the
    // shell's serial input once dispatch-level allowlist enforcement landed
    // (Phase 31b check_allowlist denies without logging).
    Read, Open, Close, Snapshot,
];

mod aliases;
mod async_utils;
mod state_transfer;
mod cmd_fs;
mod cmd_sys;
mod commands;
mod config_client;
mod executor;
mod history;
mod jobs;
mod parser;
mod shell;

use shell::ViShell;

#[no_mangle]
pub fn main() {
    let _ = ostd::syscall::sys_log("DEBUG: Shell Started (Async Mode)\n");
    let mut shell = ViShell::new();
    ostd::executor::block_on(shell.run());
}
