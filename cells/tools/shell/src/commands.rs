use ostd::fs;
use ostd::prelude::*;
use ostd::syscall;

pub fn cmd_help() -> ViResult<()> {
    crate::executor::shell_println("ViCell Shell v0.2.1 — built-in commands:");
    crate::executor::shell_println("  Files:   ls  cat  wc  head  tail  grep  find  uniq  sort  sed  mkdir  rmdir  rm");
    crate::executor::shell_println("  System:  ps  top  kill  pwd  uname  free  env  uptime  sleep  clear  exec");
    crate::executor::shell_println("  Shell:   help  echo  export  alias  unalias  jobs  source  .");
    crate::executor::shell_println("");
    crate::executor::shell_println("Syntax:  cmd | cmd2      (pipe)");
    crate::executor::shell_println("         cmd > file      (redirect stdout)");
    crate::executor::shell_println("         cmd < file      (redirect stdin)");
    crate::executor::shell_println("         cmd &           (background)");
    crate::executor::shell_println("         cmd ; cmd2      (sequence)");
    Ok(())
}

pub fn cmd_clear() -> ViResult<()> {
    ostd::io::print("\x1b[2J\x1b[1;1H"); // bypass sink — clear screen always goes to console
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
    match fs::read_dir(path) {
        Ok(iter) => {
            for entry in iter {
                let name = core::str::from_utf8(&entry.name).unwrap_or("???").trim_matches('\0');
                crate::executor::shell_println(name);
            }
            Ok(())
        }
        Err(e) => {
            ostd::io::print("ls: cannot access '"); ostd::io::print(path); ostd::io::print("': ");
            match e {
                ViError::NotFound => ostd::io::println("No such file or directory"),
                ViError::PermissionDenied => ostd::io::println("Permission denied"),
                _ => ostd::io::println("Error"),
            }
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
                                crate::executor::shell_print(s);
                                pending = 0;
                            }
                            Err(e) => {
                                let valid_len = e.valid_up_to();
                                if valid_len > 0 {
                                    let s = unsafe {
                                        core::str::from_utf8_unchecked(&buffer[..valid_len])
                                    };
                                    crate::executor::shell_print(s);
                                }

                                if let Some(error_len) = e.error_len() {
                                    crate::executor::shell_print("\u{FFFD}"); // Replacement char
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
                        if pending > 0 { crate::executor::shell_print("\u{FFFD}"); }
                        break;
                    }
                    Err(_) => {
                        ostd::io::println("cat: read error"); // error → bypass sink
                        break;
                    }
                    _ => break,
                }
            }
            syscall::sys_close(fd);
            crate::executor::shell_print("\n");
            Ok(())
        }
        Err(_) => {
            ostd::io::print("cat: "); ostd::io::print(path); ostd::io::println(": No such file or directory");
            Ok(())
        }
    }
}

pub fn cmd_ps<'a>(_args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let mut buffer = [api::syscall::ProcessInfo::default(); 16];
    match syscall::sys_get_procs(&mut buffer) {
        Ok(count) => {
            crate::executor::shell_println("PID   STATE     NAME");
            crate::executor::shell_println("------------------------");
            for i in 0..count {
                let info = &buffer[i];
                let name = core::str::from_utf8(&info.name).unwrap_or("???").trim_matches('\0');
                let state_str = match info.state { 0 => "Ready", 1 => "Running", 2 => "Waiting", 3 => "Dead", _ => "???" };
                crate::executor::shell_print(&alloc::format!("{}     {}   {}\n", info.id, state_str, name));
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
        crate::executor::shell_print(&expand_echo_escapes(&text));
    } else {
        crate::executor::shell_print(&text);
    }
    if !no_newline { crate::executor::shell_print("\n"); }
    Ok(())
}

// ─── top ──────────────────────────────────────────────────────────────────────

/// `top` — live process table, refreshed every second.
///
/// Shows PID/state/name only; CPU% is not available without per-task tick counters.
/// Press any key to exit.
pub fn cmd_top() -> ViResult<()> {
    loop {
        // Clear screen and home — always bypasses OutputSink (console control).
        ostd::io::print("\x1b[2J\x1b[1;1H");
        crate::executor::shell_println("PID   STATE     NAME");
        crate::executor::shell_println("---   --------  ----------------");
        let mut buf = [api::syscall::ProcessInfo::default(); 16];
        if let Ok(n) = syscall::sys_get_procs(&mut buf) {
            for info in &buf[..n] {
                let name = core::str::from_utf8(&info.name).unwrap_or("???").trim_matches('\0');
                let state = match info.state { 0 => "Ready", 1 => "Running", 2 => "Waiting", 3 => "Dead", _ => "???" };
                crate::executor::shell_print(&alloc::format!("{:<5} {:<9} {}\n", info.id, state, name));
            }
        }
        ostd::io::print("\n(press any key to exit)");

        // Poll for a keypress during the 1-second sleep; break on any byte.
        const HZ: u64 = 10_000_000;
        let deadline = syscall::sys_get_time().saturating_add(HZ);
        let mut got_key = false;
        while syscall::sys_get_time() < deadline {
            let mut c = [0u8; 1];
            if let Ok(n) = ostd::syscall::sys_read(0, &mut c) {
                if n > 0 { got_key = true; break; }
            }
            ostd::task::yield_now();
        }
        if got_key {
            // Drain any remaining buffered bytes (key-repeat, escape sequences)
            // so they don't leak into the next shell prompt.
            let mut drain = [0u8; 1];
            while let Ok(n) = ostd::syscall::sys_read(0, &mut drain) { if n == 0 { break; } }
            break;
        }
    }
    ostd::io::print("\x1b[2J\x1b[1;1H");
    Ok(())
}

// ─── kill ─────────────────────────────────────────────────────────────────────

/// `kill <tid>` — send a cooperative shutdown request to the target task.
///
/// Checks task state via `sys_get_procs` before sending to avoid blocking the
/// shell.  `sys_send` to a non-Recv task would put the shell in
/// `TaskState::Sending` indefinitely — so we only send when the target is
/// confirmed to be in Waiting (Recv) state.
///
/// Limitation: cannot terminate tasks blocked inside VFS/net IPC.  A kernel-level
/// `ForceExit` syscall is planned (see roadmap Phase X-6) to handle those cases.
pub fn cmd_kill<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let tid_str = match args.next() {
        Some(s) => s,
        None => { crate::executor::shell_println("Usage: kill <tid>"); return Ok(()); }
    };
    let mut tid: usize = 0;
    for ch in tid_str.bytes() {
        if !(b'0'..=b'9').contains(&ch) { crate::executor::shell_println("kill: invalid tid"); return Ok(()); }
        tid = tid.saturating_mul(10).saturating_add((ch - b'0') as usize);
    }
    if tid == 0 { crate::executor::shell_println("kill: invalid tid (0)"); return Ok(()); }

    // Safety check: only send to a task in Waiting (Recv-any) state.
    // Sending to a task in any other state blocks the shell indefinitely because
    // ipc_send puts the caller into TaskState::Sending until the target enters Recv.
    let mut procs = [api::syscall::ProcessInfo::default(); 16];
    let target_state = syscall::sys_get_procs(&mut procs).ok()
        .and_then(|n| procs[..n].iter().find(|p| p.id == tid).map(|p| p.state));

    match target_state {
        None => {
            crate::executor::shell_print(&alloc::format!("kill: no task with tid {}\n", tid));
        }
        Some(2) => {
            // Waiting state = task is in sys_recv — safe to send the signal.
            let msg = [0xFFu8];
            syscall::sys_send(tid, &msg);
            crate::executor::shell_print(&alloc::format!(
                "kill: signal sent to task {} — run 'ps' to verify termination\n", tid
            ));
        }
        Some(3) => {
            crate::executor::shell_print(&alloc::format!("kill: task {} is already Dead\n", tid));
        }
        Some(_) => {
            // Task is Ready/Running/Sleeping — cooperative signal won't reach it.
            // Use ForceExit to terminate regardless of state.
            // Kernel rejects system cells (VFS=block_io_cap, net=network_cap); use hotswap for those.
            match syscall::sys_force_exit(tid) {
                syscall::SyscallResult::Ok(_) => {
                    crate::executor::shell_print(&alloc::format!(
                        "kill: task {} force-terminated\n", tid
                    ));
                }
                syscall::SyscallResult::Err(_) => {
                    crate::executor::shell_print(&alloc::format!(
                        "kill: task {} not terminated (system cell — use hotswap; or no SpawnCap)\n",
                        tid
                    ));
                }
            }
        }
    }
    Ok(())
}
