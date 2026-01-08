#![no_std]
#![no_main]

extern crate ostd;
use ostd::prelude::*;

#[no_mangle]
pub fn main() {
    ostd::io::println("Ls (External App): Listing...");
    // Mock listing
    ostd::io::println("bin/");
    ostd::io::println("readme.txt");
    ostd::syscall::sys_exit(0);
}
