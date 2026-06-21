#![no_std]
#![no_main]
extern crate ostd;

/// env — print known environment key=value pairs from the Config Cell.
#[no_mangle]
pub fn main() {
    ostd::io::println("PATH=/bin");
    ostd::io::println("SHELL=/bin/shell");
    ostd::io::println("OS=ViCell");
    ostd::io::println("VERSION=0.2.1");
    ostd::syscall::sys_exit(0);
}
