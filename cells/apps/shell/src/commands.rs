use ostd::fs;
use ostd::prelude::*;
use ostd::syscall;

pub fn cmd_help() -> ViResult<()> {
    ostd::io::println("ViOS Shell v0.2.1 — built-in commands:");
    ostd::io::println("  Files:   ls  cat  wc  head  tail  grep  sort  sed  mkdir  rmdir  rm");
    ostd::io::println("  System:  ps  pwd  uname  free  env  uptime  sleep  clear  exec");
    ostd::io::println("  Shell:   help  echo  export  alias  unalias  jobs  source  .");
    ostd::io::println("");
    ostd::io::println("Syntax:  cmd | cmd2      (pipe)");
    ostd::io::println("         cmd > file      (redirect stdout)");
    ostd::io::println("         cmd < file      (redirect stdin)");
    ostd::io::println("         cmd &           (background)");
    ostd::io::println("         cmd ; cmd2      (sequence)");
    Ok(())
}

pub fn cmd_clear() -> ViResult<()> {
    // ANSI escape code for clear screen
    ostd::io::print("\x1b[2J\x1b[1;1H");
    Ok(())
}

pub fn cmd_exec<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let path = args.next();
    if path.is_none() {
        ostd::io::println("Usage: exec <path> [args...]");
        return Ok(());
    }
    let path = path.unwrap();

    // Reconstruct args
    let mut cmd_args = String::new();
    for arg in args {
        if !cmd_args.is_empty() {
            cmd_args.push(' ');
        }
        cmd_args.push_str(arg);
    }

    // 1. Open file using Kernel FS (same as ls/cat)
    // This ensures consistency with 'ls' and avoids relying on potentially out-of-sync Userspace VFS.
    // 1. Open file using Kernel FS
    match ostd::fs::File::open(path) {
        Ok(file) => {
            ostd::io::print("exec: loading (KERNEL-FS) ");
            ostd::io::println(path);
            exec_load_and_spawn(file, path, &cmd_args)?;
        },
        Err(_) => {
            // Fallback: Try with '/' prefix
            let mut rooted = String::from("/");
            rooted.push_str(path);
            match ostd::fs::File::open(&rooted) {
                 Ok(file) => {
                     ostd::io::print("exec: loading (KERNEL-FS) ");
                     ostd::io::println(&rooted);
                     exec_load_and_spawn(file, &rooted, &cmd_args)?;
                 },
                 Err(_) => {
                    ostd::io::print("exec: cannot open '");
                    ostd::io::print(path);
                    ostd::io::println("' (File not found)");
                 }
            }
        }
    }

    Ok(())
}

fn exec_load_and_spawn(mut file: ostd::fs::File, path: &str, cmd_args: &str) -> ViResult<()> {
    // Read file into memory
    let mut data = Vec::new();
    if let Err(_) = file.read_to_end(&mut data) {
            ostd::io::println("exec: failed to read file.");
            return Ok(());
    }

    if data.len() >= 4 {
        if data[0] != 0x7F || data[1] != 0x45 || data[2] != 0x4C || data[3] != 0x46 {
            ostd::io::println("exec: Bad ELF magic.");
            return Ok(());
        }
    }

    ostd::io::print("exec: spawning (");
    ostd::io::print_usize(data.len());
    ostd::io::println(" bytes)...");

    // Spawn
    match syscall::sys_spawn_from_mem(&data, path, cmd_args) {
        syscall::SyscallResult::Ok(tid) => {
            ostd::io::print("exec: process spawned (pid ");
            ostd::io::print_usize(tid);
            ostd::io::println(")");
            
            // Wait for it
            match syscall::sys_wait(tid) {
                syscall::SyscallResult::Ok(_) => {
                    ostd::io::println("exec: process exited.");
                }
                _ => {
                    ostd::io::println("exec: wait failed.");
                }
            }
        }
        syscall::SyscallResult::Err(_) => {
            ostd::io::println("exec: spawn failed.");
        }
    }
    Ok(())
}
// Removed IPC Logic
/*
let vfs_cell_id = 3;
*/


pub fn cmd_ls<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let path = args.next().unwrap_or("/");

    // Using ostd::fs::read_dir
    match fs::read_dir(path) {
        Ok(iter) => {
            for entry in iter {
                // entry is DirEntry
                let name = core::str::from_utf8(&entry.name).unwrap_or("???");
                // trimming null bytes
                let name = name.trim_matches('\0');
                ostd::io::println(name);
            }
            Ok(())
        }
        Err(e) => {
            // Use e to avoid unused variable warning
            ostd::io::print("ls: cannot access '");
            ostd::io::print(path);
            ostd::io::print("': ");
            match e {
                ViError::NotFound => ostd::io::println("No such file or directory"),
                ViError::PermissionDenied => ostd::io::println("Permission denied"),
                _ => ostd::io::println("Error"),
            }
            // Return Ok so shell doesn't crash on user error
            Ok(())
        }
    }
}

pub fn cmd_cat<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let path = args.next();
    if path.is_none() {
        ostd::io::println("Usage: cat <filename>");
        return Ok(());
    }
    let path = path.unwrap();

    match syscall::sys_open(path) {
        Ok(fd) => {
            let mut buffer = [0u8; 256]; // Stack buffer
            let mut pending = 0; // Number of bytes pending from previous read
            loop {
                let _max_read = buffer.len() - pending;
                match syscall::sys_read(fd, &mut buffer[pending..]) {
                    Ok(n) if n > 0 => {
                        let total = pending + n;

                        match core::str::from_utf8(&buffer[..total]) {
                            Ok(s) => {
                                ostd::io::print(s);
                                pending = 0;
                            }
                            Err(e) => {
                                let valid_len = e.valid_up_to();
                                if valid_len > 0 {
                                    let s = unsafe {
                                        core::str::from_utf8_unchecked(&buffer[..valid_len])
                                    };
                                    ostd::io::print(s);
                                }

                                if let Some(error_len) = e.error_len() {
                                    ostd::io::print("\u{FFFD}"); // Replacement char
                                    let start = valid_len + error_len;
                                    let remaining = total - start;
                                    for i in 0..remaining {
                                        buffer[i] = buffer[start + i];
                                    }
                                    pending = remaining;
                                } else {
                                    let remaining = total - valid_len;
                                    for i in 0..remaining {
                                        buffer[i] = buffer[valid_len + i];
                                    }
                                    pending = remaining;
                                }
                            }
                        }
                    }
                    Ok(0) => {
                        if pending > 0 {
                            ostd::io::print("\u{FFFD}");
                        }
                        break;
                    }
                    Err(_) => {
                        ostd::io::println("cat: read error");
                        break;
                    }
                    _ => break,
                }
            }
            syscall::sys_close(fd);
            ostd::io::println(""); // Newline at end
            Ok(())
        }
        Err(_) => {
            ostd::io::print("cat: ");
            ostd::io::print(path);
            ostd::io::println(": No such file or directory");
            // Return Ok to keep shell running
            Ok(())
        }
    }
}

pub fn cmd_ps<'a>(_args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let mut buffer = [api::syscall::ProcessInfo::default(); 16];
    match syscall::sys_get_procs(&mut buffer) {
        Ok(count) => {
            ostd::io::println("PID   STATE     NAME");
            ostd::io::println("------------------------");
            for i in 0..count {
                let info = &buffer[i];
                let name = core::str::from_utf8(&info.name).unwrap_or("???").trim_matches('\0');
                let state_str = match info.state {
                    0 => "Ready",
                    1 => "Running",
                    2 => "Waiting",
                    3 => "Dead",
                    _ => "???",
                };
                
                // Format manually since we don't have fancy formatting
                ostd::io::print_usize(info.id);
                ostd::io::print("     ");
                ostd::io::print(state_str);
                ostd::io::print("   ");
                ostd::io::println(name);
            }
            Ok(())
        }
        Err(_) => {
            ostd::io::println("ps: failed to get process list");
            Ok(())
        }
    }
}

/// Build `echo` output bytes (`"a b c\n"`) without printing.
///
/// Used by the shell redirect path to capture echo output for OP_WRITE.
pub fn cmd_echo_to_vec(args: &[&str]) -> alloc::vec::Vec<u8> {
    let mut out = alloc::vec::Vec::new();
    for (i, a) in args.iter().enumerate() {
        if i > 0 { out.push(b' '); }
        out.extend_from_slice(a.as_bytes());
    }
    out.push(b'\n');
    out
}

/// Expand `\n`, `\t`, `\\`, `\r` escape sequences in `s`.
fn expand_echo_escapes(s: &str) -> alloc::string::String {
    let mut out = alloc::string::String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n')  => out.push('\n'),
                Some('t')  => out.push('\t'),
                Some('r')  => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some('0')  => out.push('\0'),
                Some(other) => { out.push('\\'); out.push(other); }
                None        => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// `echo [-e] [-n] a b c` — print args joined by a single space.
///
/// `-e` interprets escape sequences (`\n`, `\t`, `\\`, `\r`).
/// `-n` suppresses the trailing newline.
pub fn cmd_echo<'a>(args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let parts: alloc::vec::Vec<&str> = args.collect();
    let mut escape = false;
    let mut no_newline = false;
    let mut word_start = 0;
    // Consume leading flags.
    for (i, &a) in parts.iter().enumerate() {
        if a == "-e" { escape = true; word_start = i + 1; }
        else if a == "-n" { no_newline = true; word_start = i + 1; }
        else if a == "-en" || a == "-ne" { escape = true; no_newline = true; word_start = i + 1; }
        else { break; }
    }
    let text = parts[word_start..].join(" ");
    if escape {
        ostd::io::print(&expand_echo_escapes(&text));
    } else {
        ostd::io::print(&text);
    }
    if !no_newline { ostd::io::println(""); }
    Ok(())
}
