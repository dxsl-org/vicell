//! Filesystem-oriented shell built-ins: wc, head, tail, grep, mkdir, rm, rmdir, touch.
//!
//! VFS-write operations (mkdir, rm, rmdir) send IPC to the VFS service cell at
//! `VFS_ENDPOINT`.  Read operations use the kernel's `sys_open`/`sys_read` path.

use ostd::prelude::*;
use ostd::syscall;

/// VFS service cell endpoint (task ID 3: init=1, user_hello=2, vfs=3).
const VFS_ENDPOINT: usize = 3;

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

/// Send a typed VfsRequest to the VFS cell and return whether it succeeded.
fn vfs_req_ok(req: &api::ipc::VfsRequest<'_>) -> bool {
    let mut send_buf = [0u8; 512];
    let n = match api::ipc::encode(req, &mut send_buf) {
        Ok(s) => s.len(),
        Err(_) => return false,
    };
    syscall::sys_send(VFS_ENDPOINT, &send_buf[..n]);
    let mut reply = [0u8; 64];
    match syscall::sys_recv(0, &mut reply) {
        syscall::SyscallResult::Ok(_) => {
            matches!(api::ipc::decode::<api::ipc::VfsResponse>(&reply), Ok(api::ipc::VfsResponse::Ok))
        }
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
    if vfs_req_ok(&api::ipc::VfsRequest::Mkdir(path)) {
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
    if !vfs_req_ok(&api::ipc::VfsRequest::Rmdir(path)) {
        ostd::io::print("rmdir: failed to remove '");
        ostd::io::print(path);
        ostd::io::println("' (not empty or not found)");
    }
    Ok(())
}

// ─── rm ───────────────────────────────────────────────────────────────────────

/// `rm [-r] [-f] <path>` — remove a file, or (with -r on /data) a directory tree.
pub fn cmd_rm<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let mut recursive = false;
    let path = loop {
        match args.next() {
            Some(a) if a.starts_with('-') => { recursive |= a.contains('r'); }
            Some(a) => break a,
            None => { ostd::io::println("Usage: rm [-r] <path>"); return Ok(()); }
        }
    };
    let ok = if recursive && path.starts_with("/data/") {
        rm_recursive(path)
    } else {
        vfs_req_ok(&api::ipc::VfsRequest::Unlink(path))
    };
    if !ok {
        ostd::io::print("rm: cannot remove '");
        ostd::io::print(path);
        ostd::io::println("'");
    }
    Ok(())
}

/// Recursively delete a `/data/` directory tree via VFS IPC.
pub fn rm_recursive(path: &str) -> bool {
    vfs_req_ok(&api::ipc::VfsRequest::RmdirRecursive(path))
}

/// Write `content` to `path` via typed VFS IPC.
/// The VFS server enforces `/data/`/`/tmp/` path authorization.
pub fn write_file(path: &str, content: &[u8]) -> bool {
    vfs_req_ok(&api::ipc::VfsRequest::Write { path, content })
}

/// Append `content` to `path` via typed VFS IPC.
/// Caller must chunk if content exceeds the 512-byte IPC buffer capacity.
pub fn append_file(path: &str, content: &[u8]) -> bool {
    vfs_req_ok(&api::ipc::VfsRequest::Append { path, content })
}



/// `vwrite <path> <text>` — write text to a VFS path via OP_WRITE (test helper).
pub fn cmd_vwrite<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let path = match args.next() { Some(p) => p, None => { ostd::io::println("Usage: vwrite <path> <text>"); return Ok(()); } };
    let rest = args.collect::<alloc::vec::Vec<_>>().join(" ");
    if !write_file(path, rest.as_bytes()) {
        ostd::io::print("vwrite: failed to write '");
        ostd::io::print(path);
        ostd::io::println("'");
    }
    Ok(())
}

/// `vappend <path> <text>` — append text to a VFS path via OP_APPEND (test helper).
pub fn cmd_vappend<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let path = match args.next() { Some(p) => p, None => { ostd::io::println("Usage: vappend <path> <text>"); return Ok(()); } };
    let rest = args.collect::<alloc::vec::Vec<_>>().join(" ");
    if !append_file(path, rest.as_bytes()) {
        ostd::io::print("vappend: failed to append '");
        ostd::io::print(path);
        ostd::io::println("'");
    }
    Ok(())
}

/// Read file content from VFS, trying the fast-IPC path first then falling back to ecall.
///
/// Fast path: `ostd::fast_ipc::call_vfs` calls the VFS handler directly (~3 cycles).
/// Fallback: `sys_send`/`sys_recv` round-trip via ecall (~200 cycles).
pub fn read_file_vfs(path: &str, out: &mut [u8]) -> usize {
    let req = api::ipc::VfsRequest::GetFile(path);
    let mut fast_buf = [0u8; api::ipc::IPC_BUF_SIZE];

    // Try fast-IPC path — returns 0 if VFS handler not registered yet.
    // SAFETY: fast_buf is exclusive; TrustedHandle::default() is a ZST no-op.
    let fast_n = unsafe {
        ostd::fast_ipc::call_vfs(api::fast_ipc::TrustedHandle::default(), &req, &mut fast_buf)
    };

    let decode_buf: &[u8] = if fast_n > 0 {
        &fast_buf[..fast_n]
    } else {
        // Fallback: full ecall round-trip.
        let mut send_buf = [0u8; 512];
        let n = match api::ipc::encode(&req, &mut send_buf) {
            Ok(s) => s.len(),
            Err(_) => return 0,
        };
        syscall::sys_send(VFS_ENDPOINT, &send_buf[..n]);
        match syscall::sys_recv(0, &mut fast_buf) {
            syscall::SyscallResult::Ok(_) => &fast_buf,
            _ => return 0,
        }
    };

    match api::ipc::decode::<api::ipc::VfsResponse>(decode_buf) {
        Ok(api::ipc::VfsResponse::DataPtr { ptr, len }) => {
            // SAFETY: VFS returned a pointer into its own SAS memory;
            // VFS is blocked (fast path) or waiting for next recv (ecall path).
            let data_len = (len as usize).min(out.len());
            unsafe { core::ptr::copy_nonoverlapping(ptr as *const u8, out.as_mut_ptr(), data_len); }
            data_len
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
        return Err(ViError::NotFound); // non-zero exit so `if vcat ...; then` works correctly
    }
    if let Ok(s) = core::str::from_utf8(&buf[..n]) {
        ostd::io::print(s);
    }
    Ok(())
}
