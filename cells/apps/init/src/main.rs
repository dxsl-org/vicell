#![no_std]
#![no_main]

extern crate ostd;

use ostd::io::{print, println};
use ostd::string::ToString;

#[no_mangle]
pub extern "C" fn main() {
    println("Init: Starting...");
    
    // Check math (sanity)
    if 2 + 2 == 4 {
        println("Init: Math ok.");
    }

    // Spawn Shell from Memory
    // We assume VFS has /bin/shell populated via embedded bytes
    println("Init: Loading /bin/shell...");

    // 1. Open shell file
    if let Ok(fd) = ostd::syscall::sys_open("/bin/shell") {
        // 2. Read into buffer
        let mut shell_data: ostd::vec::Vec<u8> = ostd::vec::Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            match ostd::syscall::sys_read(fd, &mut buf) {
                Ok(0) => break,
                Ok(n) => shell_data.extend_from_slice(&buf[..n]),
                Err(_) => {
                    println("Init: Error reading shell.");
                    break;
                }
            }
        }
        ostd::syscall::sys_close(fd);

        if shell_data.len() > 0 {
            print("Init: Spawning shell (size: ");
            print(&shell_data.len().to_string());
            println(")...");

            // 3. Spawn
            match ostd::syscall::sys_spawn_from_mem(&shell_data, "shell") {
                ostd::syscall::SyscallResult::Ok(_) => {
                    println("Init: Shell spawned.");
                },
                ostd::syscall::SyscallResult::Err(_) => {
                    println("Init: Failed to spawn shell (syscall error).");
                }
            }
        } else {
            println("Init: Shell file empty?");
        }
    } else {
        println("Init: Could not open /bin/shell. Is VFS working?");
    }

    // Keep init alive
    loop {
        ostd::task::yield_now();
    }
}
