#![no_std]
#![no_main]
extern crate ostd;

use ostd::{io, syscall};
use api::syscall::ProcessInfo;

/// Read the spawn-args stash into `buf` and return the trimmed content.
fn spawn_args<'a>(buf: &'a mut [u8]) -> &'a str {
    let n = syscall::sys_spawn_args(buf);
    core::str::from_utf8(&buf[..n]).unwrap_or("").trim()
}

/// kill <tid> — send cooperative shutdown signal or force-exit a task.
///
/// Cooperative (state=Waiting): sends 0xFF via sys_send; the task's recv loop
/// checks for this sentinel and exits cleanly.
/// Force (state=Ready/Running): calls sys_force_exit; the kernel terminates the
/// task immediately.  System Cells with SpawnCap may reject force-exit.
#[no_mangle]
pub fn main() {
    let mut arg_buf = [0u8; 64];
    let arg = spawn_args(&mut arg_buf);
    let tid_str = arg.split_whitespace().next().unwrap_or("");

    if tid_str.is_empty() {
        io::println("usage: kill <tid>");
        syscall::sys_exit(1);
    }

    let tid: usize = match tid_str.parse() {
        Ok(n)  => n,
        Err(_) => {
            io::print("kill: invalid tid: ");
            io::println(tid_str);
            syscall::sys_exit(1);
        }
    };

    // Read process table to determine the target task's state.
    let mut buf = [ProcessInfo::default(); 32];
    let count = syscall::sys_get_procs(&mut buf).unwrap_or(0);

    let state = (0..count)
        .find(|&i| buf[i].id == tid)
        .map(|i| buf[i].state);

    match state {
        Some(2) => {
            // Task is blocked in a recv call — send cooperative shutdown sentinel.
            let _ = syscall::sys_send(tid, &[0xFF_u8]);
            io::print("kill: sent shutdown signal to ");
            io::print_usize(tid);
            io::println("");
        }
        Some(0) | Some(1) => {
            // Task is Ready or Running — force terminate.
            let _ = syscall::sys_force_exit(tid);
            io::print("kill: force-exited ");
            io::print_usize(tid);
            io::println("");
        }
        Some(3) => {
            io::print("kill: task ");
            io::print_usize(tid);
            io::println(" is already terminated");
        }
        _ => {
            io::print("kill: task not found: ");
            io::print_usize(tid);
            io::println("");
            syscall::sys_exit(1);
        }
    }

    syscall::sys_exit(0);
}
