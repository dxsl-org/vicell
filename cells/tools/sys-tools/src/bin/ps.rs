#![no_std]
#![no_main]
extern crate ostd;
use ostd::syscall;

/// List running tasks from the kernel task table.
#[no_mangle]
pub fn main() {
    let mut buf = [api::syscall::ProcessInfo::default(); 32];
    match syscall::sys_get_procs(&mut buf) {
        Ok(count) => {
            ostd::io::println("PID   STATE     NAME");
            ostd::io::println("------------------------");
            for i in 0..count {
                let info = &buf[i];
                let name = core::str::from_utf8(&info.name)
                    .unwrap_or("?")
                    .trim_matches('\0');
                let state = match info.state {
                    0 => "Ready  ",
                    1 => "Running",
                    2 => "Waiting",
                    3 => "Dead   ",
                    _ => "?      ",
                };
                ostd::io::print_usize(info.id);
                ostd::io::print("  ");
                ostd::io::print(state);
                ostd::io::print("  ");
                ostd::io::println(name);
            }
        }
        Err(_) => ostd::io::println("ps: cannot read process table"),
    }
    syscall::sys_exit(0);
}
