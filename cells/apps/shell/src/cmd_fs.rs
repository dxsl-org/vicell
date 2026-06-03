//! Filesystem-oriented shell built-ins: wc, head, tail, grep, mkdir, rm, rmdir, touch.
//!
//! VFS-write operations (mkdir, rm, rmdir) send IPC to the VFS service cell at
//! `VFS_ENDPOINT`.  Read operations use the kernel's `sys_open`/`sys_read` path.

use ostd::prelude::*;
use ostd::syscall;

/// VFS service cell endpoint.
///
/// Boot order: init=1, user_hello=2 (kernel smoke-test), vfs=3 (init's first
/// sys_spawn_from_path). The previous value `2` silently routed all mkdir/rm
/// IPC to the user_hello task instead of the VFS service. Verified from QEMU
/// serial log — see kernel/src/task/syscall.rs ServiceLookup table.
const VFS_ENDPOINT: usize = 3;
const OP_MKDIR:  u8 = 5;
const OP_RMDIR:  u8 = 6;
const OP_UNLINK: u8 = 7;

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Read the entire contents of `path` into a Vec<u8>.
fn read_file_bytes(path: &str) -> ViResult<Vec<u8>> {
    let fd = syscall::sys_open(path).map_err(|_| ViError::NotFound)?;
    let mut bytes = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        match syscall::sys_read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => bytes.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }
    syscall::sys_close(fd);
    Ok(bytes)
}

/// Collect all newline-terminated lines from `data` into a Vec of str slices.
fn collect_lines(data: &[u8]) -> Vec<&str> {
    let text = core::str::from_utf8(data).unwrap_or("");
    text.lines().collect()
}

/// Send an OP_MKDIR/RMDIR/UNLINK IPC message to the VFS cell.
fn vfs_path_op(opcode: u8, path: &str) -> bool {
    let path_bytes = path.as_bytes();
    let path_len = path_bytes.len().min(253) as u8;
    let mut buf = [0u8; 256];
    buf[0] = opcode;
    buf[1] = path_len;
    buf[2..2 + path_len as usize].copy_from_slice(&path_bytes[..path_len as usize]);
    syscall::sys_send(VFS_ENDPOINT, &buf[..2 + path_len as usize]);
    // Receive 1-byte reply: 0=ok, non-zero=error.
    let mut reply = [0u8; 4];
    match syscall::sys_recv(0, &mut reply) {
        syscall::SyscallResult::Ok(_) => reply[0] == 0,
        _ => false,
    }
}

// ─── wc ───────────────────────────────────────────────────────────────────────

/// `wc [file]` — print line, word, and byte counts.
pub fn cmd_wc<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let path = match args.next() {
        Some(p) => p,
        None => { ostd::io::println("Usage: wc <file>"); return Ok(()); }
    };
    let data = match read_file_bytes(path) {
        Ok(d) => d,
        Err(_) => {
            ostd::io::print("wc: cannot open '");
            ostd::io::print(path);
            ostd::io::println("'");
            return Ok(());
        }
    };
    let bytes = data.len();
    let lines = data.iter().filter(|&&b| b == b'\n').count();
    let words = data.split(|b| b == &b' ' || b == &b'\n' || b == &b'\t')
        .filter(|w| !w.is_empty())
        .count();
    ostd::io::print_usize(lines);
    ostd::io::print(" ");
    ostd::io::print_usize(words);
    ostd::io::print(" ");
    ostd::io::print_usize(bytes);
    ostd::io::print(" ");
    ostd::io::println(path);
    Ok(())
}

// ─── head ─────────────────────────────────────────────────────────────────────

/// `head [-n N] <file>` — print first N lines (default 10).
pub fn cmd_head<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let mut n: usize = 10;
    let mut path = "";
    // Simple arg parsing: if first arg is "-n", consume the count.
    loop {
        match args.next() {
            Some("-n") => {
                if let Some(num) = args.next() {
                    n = num.parse().unwrap_or(10);
                }
            }
            Some(p) => { path = p; break; }
            None => break,
        }
    }
    if path.is_empty() { ostd::io::println("Usage: head [-n N] <file>"); return Ok(()); }
    let data = read_file_bytes(path).map_err(|_| { ostd::io::print("head: cannot open '"); ostd::io::print(path); ostd::io::println("'"); ViError::NotFound })?;
    for line in collect_lines(&data).into_iter().take(n) {
        ostd::io::println(line);
    }
    Ok(())
}

// ─── tail ─────────────────────────────────────────────────────────────────────

/// `tail [-n N] <file>` — print last N lines (default 10).
pub fn cmd_tail<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let mut n: usize = 10;
    let mut path = "";
    loop {
        match args.next() {
            Some("-n") => {
                if let Some(num) = args.next() { n = num.parse().unwrap_or(10); }
            }
            Some(p) => { path = p; break; }
            None => break,
        }
    }
    if path.is_empty() { ostd::io::println("Usage: tail [-n N] <file>"); return Ok(()); }
    let data = read_file_bytes(path).map_err(|_| { ostd::io::print("tail: cannot open '"); ostd::io::print(path); ostd::io::println("'"); ViError::NotFound })?;
    let lines = collect_lines(&data);
    let start = lines.len().saturating_sub(n);
    for line in &lines[start..] {
        ostd::io::println(line);
    }
    Ok(())
}

// ─── grep ─────────────────────────────────────────────────────────────────────

/// `grep [-i] <pattern> <file>` — print lines matching a literal pattern.
///
/// Uses simple substring search (no regex for v1.0; Phase 17b adds regex-lite).
pub fn cmd_grep<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let mut case_insensitive = false;
    let mut pattern = "";
    let mut path = "";

    loop {
        match args.next() {
            Some("-i") => case_insensitive = true,
            Some(p) if pattern.is_empty() => pattern = p,
            Some(p) => { path = p; break; }
            None => break,
        }
    }
    if pattern.is_empty() || path.is_empty() {
        ostd::io::println("Usage: grep [-i] <pattern> <file>");
        return Ok(());
    }
    let data = read_file_bytes(path).map_err(|_| {
        ostd::io::print("grep: cannot open '"); ostd::io::print(path); ostd::io::println("'");
        ViError::NotFound
    })?;
    let mut found = false;
    for line in collect_lines(&data) {
        let matches = if case_insensitive {
            // Byte-wise ASCII case-insensitive contains.
            contains_insensitive(line, pattern)
        } else {
            line.contains(pattern)
        };
        if matches {
            ostd::io::println(line);
            found = true;
        }
    }
    if !found {
        // Exit 1 semantics: no output.  Shell exit-code tracking in Phase 17a.
    }
    Ok(())
}

fn contains_insensitive(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() { return true; }
    let hn = needle.len();
    let hb = haystack.as_bytes();
    let nb = needle.as_bytes();
    for i in 0..hb.len().saturating_sub(hn - 1) {
        if hb[i..i + hn].iter().zip(nb).all(|(h, n)| h.to_ascii_lowercase() == n.to_ascii_lowercase()) {
            return true;
        }
    }
    false
}

// ─── mkdir ────────────────────────────────────────────────────────────────────

/// `mkdir <path>` — create a new directory via VFS IPC.
pub fn cmd_mkdir<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let path = match args.next() {
        Some(p) => p,
        None => { ostd::io::println("Usage: mkdir <path>"); return Ok(()); }
    };
    if vfs_path_op(OP_MKDIR, path) {
        // Success — silent like POSIX mkdir.
    } else {
        ostd::io::print("mkdir: cannot create directory '");
        ostd::io::print(path);
        ostd::io::println("'");
    }
    Ok(())
}

// ─── rmdir ────────────────────────────────────────────────────────────────────

/// `rmdir <path>` — remove an empty directory via VFS IPC.
pub fn cmd_rmdir<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let path = match args.next() {
        Some(p) => p,
        None => { ostd::io::println("Usage: rmdir <path>"); return Ok(()); }
    };
    if !vfs_path_op(OP_RMDIR, path) {
        ostd::io::print("rmdir: failed to remove '");
        ostd::io::print(path);
        ostd::io::println("' (not empty or not found)");
    }
    Ok(())
}

// ─── rm ───────────────────────────────────────────────────────────────────────

/// `rm <path>` — remove a file via VFS IPC (`-r` flag silently accepted).
pub fn cmd_rm<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    // Consume optional -r / -f flags.
    let path = loop {
        match args.next() {
            Some(a) if a.starts_with('-') => {}
            Some(a) => break a,
            None => { ostd::io::println("Usage: rm <path>"); return Ok(()); }
        }
    };
    if !vfs_path_op(OP_UNLINK, path) {
        ostd::io::print("rm: cannot remove '");
        ostd::io::print(path);
        ostd::io::println("'");
    }
    Ok(())
}

const OP_WRITE: u8 = 4;
const OP_READ:  u8 = 8;

/// Write `content` to `path` via VFS OP_WRITE (4-byte header:
/// opcode, path_len:u8, content_len:u16 LE). Path+content capped to 512 bytes.
/// The VFS server enforces `/data/`/`/tmp/` authorization.
pub fn write_file(path: &str, content: &[u8]) -> bool {
    let pb = path.as_bytes();
    let pl = pb.len().min(255);
    let cl = content.len().min(512_usize.saturating_sub(4 + pl));
    let mut buf = [0u8; 512];
    buf[0] = OP_WRITE;
    buf[1] = pl as u8;
    buf[2..4].copy_from_slice(&(cl as u16).to_le_bytes());
    buf[4..4 + pl].copy_from_slice(&pb[..pl]);
    buf[4 + pl..4 + pl + cl].copy_from_slice(&content[..cl]);
    syscall::sys_send(VFS_ENDPOINT, &buf[..4 + pl + cl]);
    let mut reply = [0u8; 1];
    match syscall::sys_recv(0, &mut reply) {
        syscall::SyscallResult::Ok(_) => reply[0] == 0,
        _ => false,
    }
}

/// Read file content from VFS via OP_READ; returns byte count written into `out`.
///
/// Returns 0 if the file is not found or the path is a directory.
/// Uses zero-scan to detect byte count (sys_recv returns sender_id, not length).
pub fn read_file_vfs(path: &str, out: &mut [u8]) -> usize {
    let pb = path.as_bytes();
    let pl = pb.len().min(253) as u8;
    let mut req = [0u8; 256];
    req[0] = OP_READ;
    req[1] = pl;
    req[2..2 + pl as usize].copy_from_slice(&pb[..pl as usize]);
    syscall::sys_send(VFS_ENDPOINT, &req[..2 + pl as usize]);
    match syscall::sys_recv(0, out) {
        syscall::SyscallResult::Ok(_) => {
            out.iter().rposition(|&b| b != 0).map(|i| i + 1).unwrap_or(0)
        }
        _ => 0,
    }
}

/// `vcat <path>` — print file content via VFS OP_READ (reads RamFS including /tmp/).
///
/// Unlike `cat`, which reads the kernel-embedded FS, `vcat` reads from the
/// VFS cell's RamFS — the same store that OP_WRITE targets.
pub fn cmd_vcat<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let path = match args.next() {
        Some(p) => p,
        None => { ostd::io::println("Usage: vcat <path>"); return Ok(()); }
    };
    let mut buf = [0u8; 480];
    let n = read_file_vfs(path, &mut buf);
    if n == 0 {
        ostd::io::print("vcat: not found: ");
        ostd::io::println(path);
    } else if let Ok(s) = core::str::from_utf8(&buf[..n]) {
        ostd::io::print(s);
    }
    Ok(())
}
