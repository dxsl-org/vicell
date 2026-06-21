#![no_std]
#![no_main]
extern crate ostd;

use ostd::{fs, io, syscall};

/// Read the spawn-args stash into `buf` and return the trimmed content.
fn spawn_args<'a>(buf: &'a mut [u8]) -> &'a str {
    let n = syscall::sys_spawn_args(buf);
    core::str::from_utf8(&buf[..n]).unwrap_or("").trim()
}

/// ls [path] — list kernel FS directory entries, one per line.
#[no_mangle]
pub fn main() {
    let mut arg_buf = [0u8; 256];
    let arg = spawn_args(&mut arg_buf);
    // Take the first whitespace-separated token as the path; default to "/".
    let path = arg.split_whitespace().next().unwrap_or("/");
    let path = if path.is_empty() { "/" } else { path };

    match fs::read_dir(path) {
        Ok(dir) => {
            for entry in dir {
                let name = core::str::from_utf8(&entry.name)
                    .unwrap_or("?")
                    .trim_matches('\0');
                if !name.is_empty() {
                    io::println(name);
                }
            }
        }
        Err(_) => {
            io::print("ls: ");
            io::print(path);
            io::println(": no such directory");
            syscall::sys_exit(1);
        }
    }
    syscall::sys_exit(0);
}
