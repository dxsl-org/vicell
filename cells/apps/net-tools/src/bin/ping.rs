#![no_std]
#![no_main]
extern crate ostd;

api::declare_syscalls![Log];

/// ping <host> — ICMP echo (stub; requires network + ICMP socket from Phase 15 data path).
#[no_mangle]
pub fn main() {
    ostd::io::println("ping: ICMP socket data path not yet wired (Phase 15 data-path milestone)");
    ostd::syscall::sys_exit(1);
}
