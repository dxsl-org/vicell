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
///
/// When called without a file (in a pipeline), reads from `shell_stdin()`.
pub fn cmd_wc<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let path = args.next().unwrap_or("");
    let owned;
    let data: &[u8] = if path.is_empty() {
        let s = crate::executor::shell_stdin();
        if s.is_empty() { crate::executor::shell_println("Usage: wc [file]"); return Ok(()); }
        s
    } else {
        owned = read_file_bytes(path).map_err(|_| {
            ostd::io::print("wc: cannot open '"); ostd::io::print(path); ostd::io::println("'");
            ViError::NotFound
        })?;
        &owned
    };
    let bytes = data.len();
    let lines = data.iter().filter(|&&b| b == b'\n').count();
    let words = data.split(|b| b == &b' ' || b == &b'\n' || b == &b'\t')
        .filter(|w| !w.is_empty())
        .count();
    let label = if path.is_empty() { "" } else { path };
    crate::executor::shell_print(&alloc::format!("{} {} {} {}\n", lines, words, bytes, label));
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
    if path.is_empty() { crate::executor::shell_println("Usage: head [-n N] <file>"); return Ok(()); }
    let data = read_file_bytes(path).map_err(|_| { ostd::io::print("head: cannot open '"); ostd::io::print(path); ostd::io::println("'"); ViError::NotFound })?;
    for line in collect_lines(&data).into_iter().take(n) {
        crate::executor::shell_println(line);
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
    if path.is_empty() { crate::executor::shell_println("Usage: tail [-n N] <file>"); return Ok(()); }
    let data = read_file_bytes(path).map_err(|_| { ostd::io::print("tail: cannot open '"); ostd::io::print(path); ostd::io::println("'"); ViError::NotFound })?;
    let lines = collect_lines(&data);
    let start = lines.len().saturating_sub(n);
    for line in &lines[start..] {
        crate::executor::shell_println(line);
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
    if pattern.is_empty() {
        crate::executor::shell_println("Usage: grep [-i] <pattern> [file]");
        return Ok(());
    }

    // Read from file or from pipe-fed stdin (shell_stdin()) when no path given.
    let owned;
    let data: &[u8] = if path.is_empty() {
        let s = crate::executor::shell_stdin();
        if s.is_empty() { crate::executor::shell_println("Usage: grep [-i] <pattern> [file]"); return Ok(()); }
        s
    } else {
        owned = read_file_bytes(path).map_err(|_| {
            ostd::io::print("grep: cannot open '"); ostd::io::print(path); ostd::io::println("'");
            ViError::NotFound
        })?;
        &owned
    };
    for line in collect_lines(data) {
        let matches = if case_insensitive { contains_insensitive(line, pattern) }
                      else { line.contains(pattern) };
        if matches { crate::executor::shell_println(line); }
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
    if !vfs_req_ok(&api::ipc::VfsRequest::Mkdir(path)) {
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

/// Write `data` to `path`, splitting into ≤400-byte chunks to stay within the
/// 512-byte IPC frame limit.  First chunk uses `Write` (create/overwrite);
/// subsequent chunks use `Append` to extend.  When `append` is true, every
/// chunk uses `Append`.
pub fn vfs_write_chunked(path: &str, data: &[u8], append: bool) -> bool {
    const CHUNK: usize = 400;
    if data.is_empty() {
        return if append { true } else { write_file(path, &[]) };
    }
    let mut first = !append;
    let mut ok = true;
    for chunk in data.chunks(CHUNK) {
        ok &= if first { first = false; write_file(path, chunk) } else { append_file(path, chunk) };
    }
    ok
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
        // GetFile only serves in-memory backends (RamFS) — disk-backed paths
        // (/data on FAT) have no stable pointer to hand out. Fall back to the
        // copy path: ReadAsync stores the bytes under a handle, Poll returns
        // them inline. This is how vcat reads /data since the typed migration.
        _ => read_file_vfs_async(path, out),
    }
}

/// Copy-path read via `ReadAsync` + `Poll` (≤480 bytes, the `Data` reply limit).
fn read_file_vfs_async(path: &str, out: &mut [u8]) -> usize {
    use api::ipc::{VfsRequest, VfsResponse};
    let mut buf = [0u8; 512];

    let n = match api::ipc::encode(&VfsRequest::ReadAsync { path }, &mut buf) {
        Ok(s) => s.len(),
        Err(_) => return 0,
    };
    syscall::sys_send(VFS_ENDPOINT, &buf[..n]);
    let handle = match syscall::sys_recv(0, &mut buf) {
        syscall::SyscallResult::Ok(_) => match api::ipc::decode::<VfsResponse>(&buf) {
            Ok(VfsResponse::PendingHandle(h)) => h,
            _ => return 0,
        },
        _ => return 0,
    };

    let n = match api::ipc::encode(&VfsRequest::Poll { handle }, &mut buf) {
        Ok(s) => s.len(),
        Err(_) => return 0,
    };
    syscall::sys_send(VFS_ENDPOINT, &buf[..n]);
    match syscall::sys_recv(0, &mut buf) {
        syscall::SyscallResult::Ok(_) => match api::ipc::decode::<VfsResponse>(&buf) {
            Ok(VfsResponse::Data(data)) => {
                let len = data.len().min(out.len());
                out[..len].copy_from_slice(&data[..len]);
                len
            }
            _ => 0,
        },
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
        crate::executor::shell_print(s);
    }
    Ok(())
}

// ─── find ─────────────────────────────────────────────────────────────────────

/// `find <dir> [-name pattern]` — recursively list files under `dir`.
///
/// Uses VFS `ListDir` IPC.  Directories with more than ~30 entries are silently
/// truncated by the 512-byte `ListDir` reply limit — a known v1.0 limitation.
pub fn cmd_find<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let dir = args.next().unwrap_or(".");
    let pattern = if args.next() == Some("-name") { args.next() } else { None };
    find_recursive(dir, pattern, 0);
    Ok(())
}

/// Maximum directory recursion depth for `find`.  Prevents stack overflow on
/// pathological trees; each level holds ~1 KB of stack for IPC buffers.
const FIND_MAX_DEPTH: usize = 16;

fn find_recursive(dir: &str, pattern: Option<&str>, depth: usize) {
    if depth >= FIND_MAX_DEPTH { return; }
    use api::ipc::{VfsRequest, VfsResponse};
    let mut send = [0u8; 512];
    let n = match api::ipc::encode(&VfsRequest::ListDir(dir), &mut send) {
        Ok(s) => s.len(),
        Err(_) => return,
    };
    ostd::syscall::sys_send(3, &send[..n]); // VFS_ENDPOINT = 3
    let mut reply = [0u8; 512];
    let raw = match ostd::syscall::sys_recv(0, &mut reply) {
        ostd::syscall::SyscallResult::Ok(_) => &reply,
        _ => return,
    };
    match api::ipc::decode::<VfsResponse>(raw) {
        Ok(VfsResponse::Data(entries)) => {
            let text = core::str::from_utf8(entries).unwrap_or("");
            for entry in text.lines() {
                let (kind, name) = if entry.starts_with("d:") { ("d", &entry[2..]) }
                                   else if entry.starts_with("f:") { ("f", &entry[2..]) }
                                   else { continue };
                // Build the full path without heap format for depth-zero dirs.
                let mut full = alloc::string::String::from(dir);
                if !full.ends_with('/') { full.push('/'); }
                full.push_str(name);

                if kind == "f" {
                    let matches = pattern.map(|p| name.contains(p)).unwrap_or(true);
                    if matches { crate::executor::shell_println(&full); }
                } else {
                    find_recursive(&full, pattern, depth + 1);
                }
            }
        }
        _ => {}
    }
}

// ─── uniq ─────────────────────────────────────────────────────────────────────

/// `uniq [file]` — filter adjacent duplicate lines.
///
/// When called without a file (in a pipeline), reads from `shell_stdin()`.
pub fn cmd_uniq<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let path = args.next().unwrap_or("");
    let owned;
    let data: &[u8] = if path.is_empty() {
        let s = crate::executor::shell_stdin();
        if s.is_empty() { crate::executor::shell_println("Usage: uniq [file]"); return Ok(()); }
        s
    } else {
        owned = read_file_bytes(path).map_err(|_| {
            ostd::io::print("uniq: cannot open '"); ostd::io::print(path); ostd::io::println("'");
            ViError::NotFound
        })?;
        &owned
    };
    let text = core::str::from_utf8(data).unwrap_or("");
    let mut prev = "";
    for line in text.lines() {
        if line != prev {
            crate::executor::shell_println(line);
            prev = line;
        }
    }
    Ok(())
}

// ─── sort (stdin-aware) ───────────────────────────────────────────────────────

/// `sort [file]` — sort lines lexicographically.
///
/// When called without a file (in a pipeline), reads from `shell_stdin()`.
pub fn cmd_sort<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let path = args.next().unwrap_or("");
    let owned;
    let data: &[u8] = if path.is_empty() {
        let s = crate::executor::shell_stdin();
        if s.is_empty() { crate::executor::shell_println("Usage: sort [file]"); return Ok(()); }
        s
    } else {
        owned = read_file_bytes(path).map_err(|_| {
            ostd::io::print("sort: cannot open '"); ostd::io::print(path); ostd::io::println("'");
            ViError::NotFound
        })?;
        &owned
    };
    let mut lines = collect_lines(data);
    lines.sort_unstable();
    for line in lines {
        crate::executor::shell_println(line);
    }
    Ok(())
}
