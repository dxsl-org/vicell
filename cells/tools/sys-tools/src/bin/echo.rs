#![no_std]
#![no_main]
extern crate ostd;

use ostd::{io, syscall};

/// echo [text...] — print arguments to stdout followed by a newline.
#[no_mangle]
pub fn main() {
    let mut arg_buf = [0u8; 512];
    let n = syscall::sys_spawn_args(&mut arg_buf);
    if n > 0 {
        // The args stash contains only the text after the command name.
        let text = core::str::from_utf8(&arg_buf[..n]).unwrap_or("").trim_end();
        io::print(text);
    }
    io::println("");
    syscall::sys_exit(0);
}
