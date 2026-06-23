//! Filesystem-oriented shell built-ins: wc, head, tail, grep, mkdir, rm, rmdir, touch.
//!
//! VFS-write operations (mkdir, rm, rmdir) send IPC to the VFS service cell
//! resolved via `sys_lookup_service`.  Read operations use the kernel's `sys_open`/`sys_read` path.

use ostd::prelude::*;
use ostd::syscall;

/// Resolve the live VFS service tid via the service registry.
/// Spins (yield-looping) until init has registered vfs — safe at startup
/// because init spawns vfs before shell and vfs registers before yielding.
fn vfs_endpoint() -> usize {
    use api::syscall::service;
    loop {
        if let Some(tid) = syscall::sys_lookup_service(service::VFS) {
            return tid;
        }
        ostd::task::yield_now();
    }
}

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
    syscall::sys_send(vfs_endpoint(), &send_buf[..n]);
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

/// `grep [-i] [-v] [-n] [-c] [-r] <pattern> [file|dir]` — print lines matching a literal pattern.
///
/// Flags:
///   `-i`  case-insensitive match
///   `-v`  invert: print lines that do NOT match
///   `-n`  prefix each output line with its 1-based line number
///   `-c`  count mode: print only the total count of matching lines
///   `-r`  recursive: walk a directory via VFS `ListDir` and grep each file
///
/// No regex — literal substring match only (v1.0 contract).
pub fn cmd_grep<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let mut case_insensitive = false;
    let mut invert = false;
    let mut line_numbers = false;
    let mut count_only = false;
    let mut recursive = false;
    let mut pattern = "";
    let mut path = "";

    loop {
        match args.next() {
            Some(a) if a.starts_with('-') && !a.is_empty() => {
                for ch in a[1..].chars() {
                    match ch {
                        'i' => case_insensitive = true,
                        'v' => invert = true,
                        'n' => line_numbers = true,
                        'c' => count_only = true,
                        'r' => recursive = true,
                        _   => {}
                    }
                }
            }
            Some(p) if pattern.is_empty() => pattern = p,
            Some(p) => { path = p; break; }
            None => break,
        }
    }
    if pattern.is_empty() {
        crate::executor::shell_println("Usage: grep [-ivncr] <pattern> [file|dir]");
        return Ok(());
    }

    if recursive && !path.is_empty() {
        grep_recursive(path, pattern, case_insensitive, invert, line_numbers, count_only);
        return Ok(());
    }

    // Read from file or pipe-fed stdin.
    let owned;
    let data: &[u8] = if path.is_empty() {
        let s = crate::executor::shell_stdin();
        if s.is_empty() {
            crate::executor::shell_println("Usage: grep [-ivncr] <pattern> [file|dir]");
            return Ok(());
        }
        s
    } else {
        owned = read_file_bytes(path).map_err(|_| {
            ostd::io::print("grep: cannot open '"); ostd::io::print(path); ostd::io::println("'");
            ViError::NotFound
        })?;
        &owned
    };
    grep_data(data, pattern, case_insensitive, invert, line_numbers, count_only, "");
    Ok(())
}

/// Run grep on a single data buffer; `prefix` is printed before filename-prefixed output.
fn grep_data(data: &[u8], pattern: &str, ci: bool, invert: bool,
             line_numbers: bool, count_only: bool, prefix: &str) {
    let mut hit_count: usize = 0;
    for (nr, line) in collect_lines(data).into_iter().enumerate() {
        let matches = if ci { contains_insensitive(line, pattern) }
                      else { line.contains(pattern) };
        let emit = matches ^ invert;
        if emit {
            hit_count += 1;
            if !count_only {
                if !prefix.is_empty() {
                    crate::executor::shell_print(prefix);
                    crate::executor::shell_print(":");
                }
                if line_numbers {
                    crate::executor::shell_print(
                        &alloc::format!("{}:", nr + 1)
                    );
                }
                crate::executor::shell_println(line);
            }
        }
    }
    if count_only {
        if !prefix.is_empty() {
            crate::executor::shell_print(prefix);
            crate::executor::shell_print(":");
        }
        crate::executor::shell_println(
            &alloc::format!("{}", hit_count)
        );
    }
}

/// Recursively grep all files under `dir` via VFS `ListDir` IPC.
fn grep_recursive(dir: &str, pattern: &str, ci: bool, invert: bool,
                  line_numbers: bool, count_only: bool) {
    use api::syscall::service;
    let vfs_tid = loop {
        if let Some(tid) = ostd::syscall::sys_lookup_service(service::VFS) { break tid; }
        ostd::task::yield_now();
    };
    grep_recursive_inner(dir, pattern, ci, invert, line_numbers, count_only, 0, vfs_tid);
}

const GREP_MAX_DEPTH: usize = 16;

fn grep_recursive_inner(dir: &str, pattern: &str, ci: bool, invert: bool,
                        line_numbers: bool, count_only: bool, depth: usize, vfs_tid: usize) {
    if depth >= GREP_MAX_DEPTH { return; }
    use api::ipc::{VfsRequest, VfsResponse};
    let mut send = [0u8; 512];
    let n = match api::ipc::encode(&VfsRequest::ListDir(dir), &mut send) {
        Ok(s) => s.len(), Err(_) => return,
    };
    ostd::syscall::sys_send(vfs_tid, &send[..n]);
    let mut reply = [0u8; 512];
    let raw = match ostd::syscall::sys_recv(0, &mut reply) {
        ostd::syscall::SyscallResult::Ok(_) => &reply, _ => return,
    };
    match api::ipc::decode::<VfsResponse>(raw) {
        Ok(VfsResponse::Data(entries)) => {
            let text = core::str::from_utf8(entries).unwrap_or("");
            for entry in text.lines() {
                let (kind, name) = if entry.starts_with("d:") { ("d", &entry[2..]) }
                                   else if entry.starts_with("f:") { ("f", &entry[2..]) }
                                   else { continue };
                let mut full = alloc::string::String::from(dir);
                if !full.ends_with('/') { full.push('/'); }
                full.push_str(name);
                if kind == "f" {
                    if let Ok(data) = read_file_bytes(&full) {
                        grep_data(&data, pattern, ci, invert, line_numbers, count_only, &full);
                    }
                } else {
                    grep_recursive_inner(&full, pattern, ci, invert, line_numbers, count_only, depth + 1, vfs_tid);
                }
            }
        }
        _ => {}
    }
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
        syscall::sys_send(vfs_endpoint(), &send_buf[..n]);
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
    syscall::sys_send(vfs_endpoint(), &buf[..n]);
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
    syscall::sys_send(vfs_endpoint(), &buf[..n]);
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
    // Resolve once; pass TID through recursion to avoid a syscall per directory level.
    let vfs_tid = vfs_endpoint();
    find_recursive(dir, pattern, 0, vfs_tid);
    Ok(())
}

/// Maximum directory recursion depth for `find`.  Prevents stack overflow on
/// pathological trees; each level holds ~1 KB of stack for IPC buffers.
const FIND_MAX_DEPTH: usize = 16;

fn find_recursive(dir: &str, pattern: Option<&str>, depth: usize, vfs_tid: usize) {
    if depth >= FIND_MAX_DEPTH { return; }
    use api::ipc::{VfsRequest, VfsResponse};
    let mut send = [0u8; 512];
    let n = match api::ipc::encode(&VfsRequest::ListDir(dir), &mut send) {
        Ok(s) => s.len(),
        Err(_) => return,
    };
    ostd::syscall::sys_send(vfs_tid, &send[..n]);
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
                    find_recursive(&full, pattern, depth + 1, vfs_tid);
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

// ─── tee ─────────────────────────────────────────────────────────────────────

/// `tee [-a] <path>` — read stdin, write to both stdout sink and a VFS file.
///
/// `-a` appends to the file instead of overwriting. Data flows through the
/// shell pipeline (via `shell_print`) AND is written to `path` via VFS IPC.
pub fn cmd_tee<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let mut append = false;
    let path = loop {
        match args.next() {
            Some("-a") => append = true,
            Some(p)    => break p,
            None       => {
                crate::executor::shell_println("Usage: tee [-a] <path>");
                return Ok(());
            }
        }
    };
    let data = crate::executor::shell_stdin();
    if data.is_empty() {
        // No stdin and no pipeline data: nothing to tee.
        return Ok(());
    }
    // Write to the current output sink (console or outer pipeline capture).
    if let Ok(s) = core::str::from_utf8(data) {
        crate::executor::shell_print(s);
    }
    // Also write the same data to the VFS file.
    if !vfs_write_chunked(path, data, append) {
        ostd::io::print("tee: cannot write '");
        ostd::io::print(path);
        ostd::io::println("'");
    }
    Ok(())
}

// ─── sed ─────────────────────────────────────────────────────────────────────

/// `sed [-n] EXPR [file]` — stream editor: substitute, delete, or print lines.
///
/// Supported expression forms (no regex — literal match only):
///   `s/PAT/REP/[g]`  — replace PAT with REP (first occurrence, or all with `g`)
///   `/PAT/d`         — delete (suppress) lines matching PAT
///   `/PAT/p`         — print lines matching PAT (most useful with `-n`)
///   `Np`             — print only line N (1-based)
///
/// `-n` suppresses the default auto-print; only explicitly `p`-addressed lines appear.
/// Reads from `shell_stdin()` when no file is given (pipeline-friendly).
pub fn cmd_sed<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    // Parse leading -n flag.
    let mut suppress = false;
    let expr = loop {
        match args.next() {
            Some("-n") => suppress = true,
            Some(e)    => break e,
            None => {
                crate::executor::shell_println(
                    "Usage: sed [-n] s/PAT/REP/[g] | /PAT/d | /PAT/p | Np  [file]"
                );
                return Ok(());
            }
        }
    };

    // Classify the expression.
    enum SedOp<'x> {
        Substitute { pat: &'x str, rep: &'x str, global: bool },
        Delete(&'x str),    // pattern to match for deletion
        Print(SedAddr<'x>), // print matching lines
    }
    enum SedAddr<'x> { Pattern(&'x str), LineNum(usize) }

    let op: SedOp = if let Some(body) = expr.strip_prefix("s/") {
        let mut parts = body.splitn(3, '/');
        let pat   = parts.next().unwrap_or("");
        let rep   = parts.next().unwrap_or("");
        let flags = parts.next().unwrap_or("");
        SedOp::Substitute { pat, rep, global: flags.contains('g') }
    } else if expr.starts_with('/') && expr.ends_with('d') {
        let inner = &expr[1..];
        let pat = inner.trim_end_matches('/').trim_end_matches('d')
                       .trim_end_matches('/');
        SedOp::Delete(pat)
    } else if expr.starts_with('/') && (expr.ends_with('p') || expr.ends_with("/p")) {
        let inner = &expr[1..];
        let pat = inner.trim_end_matches('/').trim_end_matches('p')
                       .trim_end_matches('/');
        SedOp::Print(SedAddr::Pattern(pat))
    } else if expr.ends_with('p') && expr[..expr.len()-1].bytes().all(|b| b.is_ascii_digit()) {
        let n = expr[..expr.len()-1].parse::<usize>().unwrap_or(0);
        SedOp::Print(SedAddr::LineNum(n))
    } else {
        crate::executor::shell_print("sed: unrecognised expression: ");
        crate::executor::shell_println(expr);
        return Ok(());
    };

    // Optional file argument.
    let path = args.next().unwrap_or("");
    let owned;
    let data: &[u8] = if path.is_empty() {
        let s = crate::executor::shell_stdin();
        if s.is_empty() {
            crate::executor::shell_println(
                "Usage: sed [-n] s/PAT/REP/[g] | /PAT/d | /PAT/p | Np  [file]"
            );
            return Ok(());
        }
        s
    } else {
        owned = read_file_bytes(path).map_err(|_| {
            ostd::io::print("sed: cannot open '");
            ostd::io::print(path);
            ostd::io::println("'");
            ViError::NotFound
        })?;
        &owned
    };

    let text = core::str::from_utf8(data).unwrap_or("");

    for (idx, line) in text.lines().enumerate() {
        let nr = idx + 1; // 1-based
        match &op {
            SedOp::Substitute { pat, rep, global } => {
                if pat.is_empty() {
                    if !suppress { crate::executor::shell_println(line); }
                } else {
                    let out = if *global { sed_replace_all(line, pat, rep) }
                              else       { sed_replace_first(line, pat, rep) };
                    if !suppress { crate::executor::shell_println(&out); }
                }
            }
            SedOp::Delete(pat) => {
                let matches = line.contains(*pat);
                if !matches && !suppress { crate::executor::shell_println(line); }
            }
            SedOp::Print(addr) => {
                let matches = match addr {
                    SedAddr::Pattern(p) => line.contains(*p),
                    SedAddr::LineNum(n) => nr == *n,
                };
                // With `-n`: only explicit `p` prints; without `-n`: also auto-print
                // every line, so matched lines appear twice (POSIX sed semantics).
                if matches    { crate::executor::shell_println(line); }
                if !suppress  { crate::executor::shell_println(line); }
            }
        }
    }
    Ok(())
}

/// Replace the first occurrence of `pat` in `s` with `rep`.
// ─── awk ─────────────────────────────────────────────────────────────────────

/// `awk [-F sep] [/pattern/] [col,...] [file]` — field extractor and line filter.
///
/// Because the shell tokenizer treats `{` and `}` as syntax operators, the
/// standard `awk '{print $1}'` form cannot be passed intact.  This implementation
/// uses a shell-friendly syntax instead:
///
/// - `-F sep`      — single-character field separator (default: whitespace).
/// - `/pattern/`   — print only lines containing the literal pattern.
/// - `col,...`     — comma-separated 1-based column indices to print (0 = full line).
///                   Omit to print the entire matching line.
/// - `file`        — path to read; reads `shell_stdin()` when absent.
///
/// Examples:
///   `awk -F: 1`           — first `:` -delimited field on each line
///   `awk /error/ 1 3`     — fields 1 and 3 from lines containing "error"
///   `awk 0`               — passthrough (entire lines)
///   `ps | awk /Running/ 2` — pipe-friendly
pub fn cmd_awk<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let mut sep: Option<char> = None;
    let mut pattern = "";
    let mut cols = [0usize; 8];
    let mut ncols: usize = 0;
    let mut path = "";

    loop {
        match args.next() {
            Some("-F") => {
                match args.next() {
                    Some(s) => sep = s.chars().next(),
                    None => {
                        crate::executor::shell_println("awk: -F requires a separator character");
                        return Ok(());
                    }
                }
            }
            Some(a) if a.starts_with("-F") && a.len() > 2 => {
                sep = a[2..].chars().next();
            }
            // /pattern/ — starts and ends with '/' with no inner '/'
            Some(a) if a.len() >= 3 && a.starts_with('/') && a.ends_with('/')
                    && !a[1..a.len()-1].contains('/') => {
                pattern = &a[1..a.len()-1];
            }
            // col,col,... — non-empty, all digits or commas
            Some(a) if !a.is_empty()
                    && a.bytes().all(|b| b.is_ascii_digit() || b == b',')
                    && !a.starts_with(',') && !a.ends_with(',') => {
                for part in a.split(',') {
                    if let Ok(n) = part.parse::<usize>() {
                        if ncols < 8 { cols[ncols] = n; ncols += 1; }
                    }
                }
            }
            Some(a) => { path = a; break; }
            None     => break,
        }
    }

    let owned;
    let data: &[u8] = if path.is_empty() {
        let s = crate::executor::shell_stdin();
        if s.is_empty() {
            crate::executor::shell_println(
                "Usage: awk [-F sep] [/pattern/] [col,...] [file]"
            );
            return Ok(());
        }
        s
    } else {
        owned = read_file_bytes(path).map_err(|_| {
            ostd::io::print("awk: cannot open '");
            ostd::io::print(path);
            ostd::io::println("'");
            ViError::NotFound
        })?;
        &owned
    };

    let text = core::str::from_utf8(data).unwrap_or("");

    for line in text.lines() {
        if !pattern.is_empty() && !line.contains(pattern) { continue; }

        if ncols == 0 {
            crate::executor::shell_println(line);
        } else {
            let fields: alloc::vec::Vec<&str> = if let Some(s) = sep {
                line.split(s).collect()
            } else {
                line.split_whitespace().collect()
            };
            let mut first_col = true;
            for i in 0..ncols {
                let col = cols[i];
                let val: &str = if col == 0 {
                    line
                } else {
                    fields.get(col - 1).copied().unwrap_or("")
                };
                if !first_col { crate::executor::shell_print(" "); }
                crate::executor::shell_print(val);
                first_col = false;
            }
            crate::executor::shell_print("\n");
        }
    }
    Ok(())
}

fn sed_replace_first(s: &str, pat: &str, rep: &str) -> String {
    match s.find(pat) {
        Some(i) => {
            let mut out = String::with_capacity(s.len() + rep.len());
            out.push_str(&s[..i]);
            out.push_str(rep);
            out.push_str(&s[i + pat.len()..]);
            out
        }
        None => String::from(s),
    }
}

/// Replace all non-overlapping occurrences of `pat` in `s` with `rep`.
fn sed_replace_all(s: &str, pat: &str, rep: &str) -> String {
    let mut out = String::with_capacity(s.len() + rep.len());
    let mut rest = s;
    while let Some(i) = rest.find(pat) {
        out.push_str(&rest[..i]);
        out.push_str(rep);
        rest = &rest[i + pat.len()..];
    }
    out.push_str(rest);
    out
}
