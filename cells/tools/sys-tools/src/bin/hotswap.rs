#![no_std]
#![no_main]
extern crate ostd;
extern crate alloc;

use alloc::string::String;
use ostd::syscall;

/// hotswap <cell-name> <new-elf-path> — live-upgrade a running Cell.
///
/// Looks up the cell by name via `ps` output, then calls `sys_hotswap`.
///
/// Usage:
///   hotswap shell /bin/shell-v2
///   hotswap config /bin/config-v2
///   hotswap vfs /bin/vfs-v2
///
/// The swap preserves in-flight IPC messages and serialised cell state.
/// Cells must implement `api::hotswap::ViStateTransfer` to survive a swap.
///
/// Since arg-passing is not yet wired (Phase 17a), this binary reads
/// cell_name and new_elf_path from stdin (one per line) as a workaround.
#[no_mangle]
pub fn main() {
    ostd::io::println("hotswap — live cell upgrade");
    ostd::io::println("Enter: <cell-name>");
    let cell_name = read_line();
    ostd::io::println("Enter: <new-elf-path>");
    let new_path = read_line();

    if cell_name.is_empty() || new_path.is_empty() {
        ostd::io::println("hotswap: usage: hotswap <cell-name> <new-elf-path>");
        syscall::sys_exit(1);
    }

    // Find the cell_id by name from the process table.
    let mut buf = [api::syscall::ProcessInfo::default(); 32];
    let cell_id = match syscall::sys_get_procs(&mut buf) {
        Ok(count) => {
            let mut found = None;
            for i in 0..count {
                let name = core::str::from_utf8(&buf[i].name)
                    .unwrap_or("")
                    .trim_matches('\0');
                if name == cell_name.as_str() {
                    found = Some(buf[i].id);
                    break;
                }
            }
            found
        }
        Err(_) => {
            ostd::io::println("hotswap: cannot read process table");
            syscall::sys_exit(1);
        }
    };

    let cell_id = match cell_id {
        Some(id) => id,
        None => {
            ostd::io::print("hotswap: cell '");
            ostd::io::print(&cell_name);
            ostd::io::println("' not found");
            syscall::sys_exit(1);
        }
    };

    ostd::io::print("hotswap: upgrading cell ");
    ostd::io::print_usize(cell_id);
    ostd::io::print(" to ");
    ostd::io::println(&new_path);

    match syscall::sys_hotswap(cell_id, &new_path) {
        syscall::SyscallResult::Ok(new_id) => {
            ostd::io::print("hotswap: done — new cell task id ");
            ostd::io::print_usize(new_id);
            ostd::io::println("");
        }
        syscall::SyscallResult::Err(_) => {
            ostd::io::println("hotswap: swap failed (see kernel log)");
            syscall::sys_exit(1);
        }
    }
    syscall::sys_exit(0);
}

/// Read a line from stdin into an owned String.
fn read_line() -> String {
    let mut s = String::new();
    let mut buf = [0u8; 1];
    loop {
        match syscall::sys_read(0, &mut buf) {
            Ok(1) if buf[0] != b'\n' && buf[0] != b'\r' => {
                s.push(buf[0] as char);
            }
            _ => break,
        }
    }
    s
}
