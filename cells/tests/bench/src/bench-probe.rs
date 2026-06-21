#![no_std]
#![no_main]
#![allow(dead_code)] // orchestrator-only items in shared modules are unreachable from this binary

extern crate alloc;

mod framework;
mod scenarios;

// Probe/load roles do not spawn further children.
api::declare_syscalls![Send, Recv, Log, GetTime, Heartbeat, SetTimer, StateRestore, Exit, Yield];
api::declare_manifest!(block_io = false, network = false, spawn = false);

#[no_mangle]
pub fn main() {
    let mut argbuf = [0u8; 32];
    let an = ostd::syscall::sys_spawn_args(&mut argbuf);
    let role = core::str::from_utf8(&argbuf[..an]).unwrap_or("");
    ostd::io::println(&alloc::format!("[bench-probe] Started with role: '{}'", role));
    match role {
        "load"     => scenarios::rt_load::run_load(),
        "rt-probe" => scenarios::preempt_latency::run_probe(),
        "ctl-loop" => scenarios::control_loop::run_control_loop(),
        "ipc-echo" => {
            let mut buf = [0u8; 64];
            loop {
                let sender = match ostd::syscall::sys_recv(0, &mut buf) {
                    ostd::syscall::SyscallResult::Ok(sid) => sid,
                    _ => continue,
                };
                ostd::syscall::sys_send(sender, &[0]);
            }
        }
        "smp-worker" => scenarios::smp::run_worker(),
        _ => {} // unknown role: exit cleanly
    }
}
