#![no_std]
#![no_main]
extern crate ostd;

use ostd::{io, syscall};

/// Read the spawn-args stash into `buf` and return the trimmed content.
fn spawn_args<'a>(buf: &'a mut [u8]) -> &'a str {
    let n = syscall::sys_spawn_args(buf);
    core::str::from_utf8(&buf[..n]).unwrap_or("").trim()
}

/// cat <path> — print file content from the kernel FS to stdout.
#[no_mangle]
pub fn main() {
    let mut arg_buf = [0u8; 256];
    let arg = spawn_args(&mut arg_buf);
    let path = arg.split_whitespace().next().unwrap_or("");

    if path.is_empty() {
        io::println("usage: cat <path>");
        syscall::sys_exit(1);
    }

    let fd = match syscall::sys_open(path) {
        Ok(f)  => f,
        Err(_) => {
            io::print("cat: ");
            io::print(path);
            io::println(": file not found");
            syscall::sys_exit(1);
        }
    };

    let mut buf = [0u8; 512];
    loop {
        match syscall::sys_read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if let Ok(s) = core::str::from_utf8(&buf[..n]) {
                    io::print(s);
                }
            }
            Err(_) => break,
        }
    }
    syscall::sys_close(fd);
    syscall::sys_exit(0);
}
