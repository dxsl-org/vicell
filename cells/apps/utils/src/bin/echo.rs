#![no_std]
#![no_main]

extern crate ostd;
use ostd::prelude::*;

#[no_mangle]
pub fn main() {
    // Echo: print args joined by space
    // We don't have args parsing in main yet (Kernel passes arg via Regs? Or not implemented).
    // For now, simple echo.
    ostd::io::println("Echo (External App): Hello!");
    ostd::syscall::sys_exit(0);
}
