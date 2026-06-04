//! Shell AST executor — runs parsed commands, handles pipes and redirects.
//!
//! Pipes between cells in ViOS v1.0 are simulated via IPC-based streaming:
//! the first command's output is buffered in a `Vec<u8>`, then fed as stdin
//! to the next command.  Full zero-copy IPC pipes are deferred to Phase 17a
//! when the capability pipe primitive lands.

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use crate::parser::{Ast, Cmd, Redirect};
use crate::jobs::{Jobs, JobState};
use ostd::prelude::*;
use ostd::syscall;

// ── Shell function store ──────────────────────────────────────────────────────
//
// Functions are stored as (name, body_text) pairs.  When a command name matches
// a stored function, its body text is re-parsed and executed in the current
// shell context (same Jobs, same VARS store).

const MAX_FUNS: usize = 8;

static mut FUNS: [(bool, [u8; 32], [u8; 480]); MAX_FUNS] =
    [(false, [0u8; 32], [0u8; 480]); MAX_FUNS];

pub fn define_function(name: &str, body: &str) {
    let nb = name.as_bytes();
    let bb = body.as_bytes();
    let nlen = nb.len().min(31);
    let blen = bb.len().min(479);
    // SAFETY: single shell task; no concurrent writes.
    let store = unsafe { &mut FUNS };
    // Update existing.
    for slot in store.iter_mut() {
        if slot.0 && &slot.1[..nlen] == &nb[..nlen] && slot.1[nlen] == 0 {
            slot.2[..blen].copy_from_slice(&bb[..blen]);
            slot.2[blen] = 0;
            return;
        }
    }
    // First empty slot.
    for slot in store.iter_mut() {
        if !slot.0 {
            slot.0 = true;
            slot.1[..nlen].copy_from_slice(&nb[..nlen]);
            slot.1[nlen] = 0;
            slot.2[..blen].copy_from_slice(&bb[..blen]);
            slot.2[blen] = 0;
            return;
        }
    }
}

fn get_function(name: &str) -> Option<&'static str> {
    let nb = name.as_bytes();
    let nlen = nb.len().min(31);
    // SAFETY: single shell task; no concurrent reads.
    let store = unsafe { &FUNS };
    for slot in store.iter() {
        if slot.0 && &slot.1[..nlen] == &nb[..nlen] && slot.1[nlen] == 0 {
            let blen = slot.2.iter().position(|&b| b == 0).unwrap_or(480);
            return core::str::from_utf8(&slot.2[..blen]).ok();
        }
    }
    None
}

// ── Shell exit signal ─────────────────────────────────────────────────────────
//
// `exit [N]` sets this flag; the shell's main run() loop checks it after each
// command and terminates when set.  Single-threaded — static is safe.

static mut EXIT_REQUESTED: bool = false;
static mut EXIT_CODE_VALUE: i32  = 0;

/// Signal the shell to exit with the given code on its next loop iteration.
pub fn request_exit(code: i32) {
    // SAFETY: single shell task; no concurrent writes.
    unsafe { EXIT_REQUESTED = true; EXIT_CODE_VALUE = code; }
}

/// True if `exit` has been called; clears the flag.
pub fn take_exit_request() -> Option<i32> {
    // SAFETY: single shell task; no concurrent reads/writes.
    unsafe {
        if EXIT_REQUESTED {
            EXIT_REQUESTED = false;
            Some(EXIT_CODE_VALUE)
        } else {
            None
        }
    }
}

// ── Loop control signal ───────────────────────────────────────────────────────
//
// `break` and `continue` built-ins set a static signal that the nearest
// enclosing while/for executor arm consumes.  The shell is single-threaded
// (one task, cooperative scheduling) so a static flag is safe.

#[derive(Clone, Copy, PartialEq, Eq)]
enum LoopSignal { None, Break, Continue }

static mut LOOP_SIGNAL: LoopSignal = LoopSignal::None;

pub fn set_loop_signal(s: LoopSignal) {
    // SAFETY: single shell task; no concurrent writes.
    unsafe { LOOP_SIGNAL = s; }
}

fn take_loop_signal() -> LoopSignal {
    // SAFETY: single shell task; no concurrent access.
    unsafe {
        let s = LOOP_SIGNAL;
        LOOP_SIGNAL = LoopSignal::None;
        s
    }
}

// ── Shell variable store ──────────────────────────────────────────────────────
//
// Up to 16 named variables, keyed as fixed-width byte arrays.
// The shell runs as a single task with no concurrent access, so a static
// array is safe here.  Lifetimes of values returned by get_var are bounded
// to the next set_var call — callers must not keep references across mutations.

const MAX_VARS: usize = 16;

// Slot layout: (occupied, key[32], value[128]).  NUL-terminated on set.
static mut VARS: [(bool, [u8; 32], [u8; 128]); MAX_VARS] =
    [(false, [0u8; 32], [0u8; 128]); MAX_VARS];

fn unset_var(key: &str) {
    let kb = key.as_bytes();
    let klen = kb.len().min(31);
    // SAFETY: single shell task; no concurrent writes.
    let store = unsafe { &mut VARS };
    for slot in store.iter_mut() {
        if slot.0 && slot.1[..klen] == kb[..klen] && slot.1[klen] == 0 {
            slot.0 = false;
            return;
        }
    }
}

fn set_var(key: &str, value: &str) {
    let kb = key.as_bytes();
    let vb = value.as_bytes();
    // SAFETY: single shell task; no concurrent writes to VARS.
    let store = unsafe { &mut VARS };
    let klen = kb.len().min(31);
    let vlen = vb.len().min(127);
    // Update existing slot first.
    for slot in store.iter_mut() {
        if slot.0 && slot.1[..klen] == kb[..klen] && slot.1[klen] == 0 {
            slot.2[..vlen].copy_from_slice(&vb[..vlen]);
            slot.2[vlen] = 0;
            return;
        }
    }
    // Use first empty slot.
    for slot in store.iter_mut() {
        if !slot.0 {
            slot.0 = true;
            slot.1[..klen].copy_from_slice(&kb[..klen]);
            slot.1[klen] = 0;
            slot.2[..vlen].copy_from_slice(&vb[..vlen]);
            slot.2[vlen] = 0;
            return;
        }
    }
    // Store full — silently drop. 16 variables is sufficient for scripts.
}

fn get_var(key: &str) -> Option<&'static str> {
    let kb = key.as_bytes();
    let klen = kb.len().min(31);
    // SAFETY: single shell task; no concurrent reads.
    let store = unsafe { &VARS };
    for slot in store.iter() {
        if slot.0 && slot.1[..klen] == kb[..klen] && slot.1[klen] == 0 {
            let vlen = slot.2.iter().position(|&b| b == 0).unwrap_or(128);
            return core::str::from_utf8(&slot.2[..vlen]).ok();
        }
    }
    None
}

/// Expand a single token: `$NAME` (whole-token only) → variable value.
/// Non-`$` tokens are returned unchanged (as an owned String clone).
/// Expand `$VAR` and `$?` references anywhere inside a token (mid-token expansion).
///
/// Scans for `$` followed by an identifier (`[A-Za-z_][A-Za-z0-9_]*`) or `?`.
/// Any `$` that is not followed by a valid name is passed through unchanged.
/// Fast path: tokens with no `$` are returned as-is (no allocation).
fn expand_token(s: &str) -> String {
    if !s.contains('$') { return String::from(s); }
    let mut result = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            if next == b'?' {
                // $? — exit code of the last command.
                if let Some(v) = get_var("?") { result.push_str(v); }
                i += 2;
                continue;
            }
            if next == b'#' {
                // $# — positional argument count.
                if let Some(v) = get_var("#") { result.push_str(v); }
                i += 2;
                continue;
            }
            if next == b'@' {
                // $@ — all positional arguments joined with spaces.
                if let Some(v) = get_var("@") { result.push_str(v); }
                i += 2;
                continue;
            }
            if next.is_ascii_digit() && next != b'0' {
                // $1..$9 — single-digit positional parameter.
                let key = unsafe { core::str::from_utf8_unchecked(&bytes[i+1..i+2]) };
                if let Some(v) = get_var(key) { result.push_str(v); }
                i += 2;
                continue;
            }
            if next.is_ascii_alphabetic() || next == b'_' {
                let start = i + 1;
                let end   = bytes[start..].iter()
                    .take_while(|&&b| b.is_ascii_alphanumeric() || b == b'_')
                    .count() + start;
                // SAFETY: bytes[start..end] contains only ASCII alphanumeric / '_'.
                let name = unsafe { core::str::from_utf8_unchecked(&bytes[start..end]) };
                if let Some(v) = get_var(name) { result.push_str(v); }
                // Unset variables expand to empty string (POSIX default).
                i = end;
                continue;
            }
        }
        result.push(bytes[i] as char); // shell tokens are ASCII
        i += 1;
    }
    result
}

/// Execute an `Ast` and return the last command's exit code.
///
/// `stdin_data` is the bytes available on stdin for the first command in a pipeline.
pub fn execute(ast: &Ast, jobs: &mut Jobs) -> i32 {
    match ast {
        Ast::Empty => 0,
        Ast::Simple(cmd) => exec_cmd(cmd, &[], jobs),
        Ast::Pipeline(cmds) => exec_pipeline(cmds, jobs),
        Ast::Background(cmd) => {
            // Spawn the command as a background job without waiting.
            let name = cmd.argv.first().map(String::as_str).unwrap_or("?");
            let jid = jobs.add(name);
            ostd::io::print("[");
            ostd::io::print_usize(jid);
            ostd::io::println("] spawning background job");
            // For v1.0: spawn and don't wait (background support is basic).
            exec_cmd(cmd, &[], jobs);
            jobs.set_state(jid, JobState::Done);
            0
        }
        Ast::Sequence(sub) => {
            let mut last = 0;
            for s in sub {
                last = execute(s, jobs);
            }
            last
        }
        Ast::Case { expr, arms } => {
            let value = expand_token(expr);
            for (pattern, body) in arms {
                if case_matches(pattern, &value) {
                    execute(body, jobs);
                    break;
                }
            }
            0
        }
        Ast::FuncDef { name, body } => {
            define_function(name, body);
            0
        }
        Ast::And(left, right) => {
            let code = execute(left, jobs);
            if code == 0 { execute(right, jobs) } else { code }
        }
        Ast::Or(left, right) => {
            let code = execute(left, jobs);
            if code != 0 { execute(right, jobs) } else { code }
        }
        Ast::While { cond, body } => {
            loop {
                if execute(cond, jobs) != 0 { break; }
                execute(body, jobs);
                match take_loop_signal() {
                    LoopSignal::Break    => break,
                    LoopSignal::Continue => continue,
                    LoopSignal::None     => {}
                }
            }
            0
        }
        Ast::For { var, words, body } => {
            'for_loop: for word in words {
                set_var(var, word);
                execute(body, jobs);
                match take_loop_signal() {
                    LoopSignal::Break    => break 'for_loop,
                    LoopSignal::Continue => continue 'for_loop,
                    LoopSignal::None     => {}
                }
            }
            0
        }
        Ast::If { cond, then_b, else_b } => {
            let code = execute(cond, jobs);
            if code == 0 {
                execute(then_b, jobs)
            } else if let Some(eb) = else_b {
                execute(eb, jobs)
            } else {
                0
            }
        }
    }
}

/// Execute a pipeline: run each command in order, piping stdout→stdin.
fn exec_pipeline(cmds: &[Cmd], jobs: &mut Jobs) -> i32 {
    let mut stdin_data: Vec<u8> = Vec::new();
    let mut exit = 0;
    for cmd in cmds {
        let out = capture_cmd(cmd, &stdin_data, jobs);
        // Print the last stage's output (unless it will be piped further).
        let is_last = core::ptr::eq(cmd, cmds.last().unwrap());
        if is_last {
            if let Ok(s) = core::str::from_utf8(&out) {
                ostd::io::print(s);
            }
        }
        exit = 0;
        stdin_data = out;
    }
    exit
}

/// Run a command, capturing its output into a `Vec<u8>`.
///
/// For built-in commands this is approximate — v1.0 captures via in-memory
/// buffer; external binaries would need pipe caps (Phase 17a).
fn capture_cmd(cmd: &Cmd, _stdin: &[u8], jobs: &mut Jobs) -> Vec<u8> {
    // For now, run as a simple command and return empty bytes.
    // Full capture requires spawning + pipe caps.
    exec_cmd(cmd, _stdin, jobs);
    Vec::new()
}

/// Execute one simple command.
///
/// Handles redirection, built-in dispatch, and external binary spawn.
fn exec_cmd(cmd: &Cmd, _stdin: &[u8], jobs: &mut Jobs) -> i32 {
    if cmd.is_empty() { return 0; }

    // Expand $VAR tokens in every argument before dispatch.
    let expanded: Vec<String> = cmd.argv.iter().map(|s| expand_token(s)).collect();
    let prog: &str = &expanded[0];
    let args: Vec<&str> = expanded[1..].iter().map(String::as_str).collect();

    // Detect `KEY=VALUE` assignment (key is non-empty alphanumeric+underscore).
    if args.is_empty() {
        if let Some(eq) = prog.find('=') {
            let key = &prog[..eq];
            if !key.is_empty() && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                set_var(key, &prog[eq + 1..]);
                return 0;
            }
        }
    }

    // Capture `echo` output for `>` (overwrite) and `>>` (append) redirects.
    // External-process capture requires pipe caps (Phase 17a) and is out of scope.
    if prog == "echo" {
        if let Some(Redirect::StdoutTo(path)) =
            cmd.redirects.iter().find(|r| matches!(r, Redirect::StdoutTo(_)))
        {
            let bytes = crate::commands::cmd_echo_to_vec(&args);
            if !crate::cmd_fs::write_file(path, &bytes) {
                ostd::io::print("echo: cannot write '");
                ostd::io::print(path);
                ostd::io::println("'");
            }
            return 0;
        }
        if let Some(Redirect::StdoutAppend(path)) =
            cmd.redirects.iter().find(|r| matches!(r, Redirect::StdoutAppend(_)))
        {
            let bytes = crate::commands::cmd_echo_to_vec(&args);
            if !crate::cmd_fs::append_file(path, &bytes) {
                ostd::io::print("echo: cannot append '");
                ostd::io::print(path);
                ostd::io::println("'");
            }
            return 0;
        }
    }

    // Handle remaining redirects.  `StdinFrom` prints file content inline
    // (Phase V scope — full stdin plumbing deferred to Phase 17a pipe caps).
    for r in &cmd.redirects {
        match r {
            Redirect::StdinFrom(path) => {
                let mut buf = alloc::vec![0u8; 4096];
                let n = crate::cmd_fs::read_file_vfs(path, &mut buf);
                if n > 0 {
                    if let Ok(s) = core::str::from_utf8(&buf[..n]) {
                        ostd::io::print(s);
                    }
                } else {
                    ostd::io::print("shell: cannot open '");
                    ostd::io::print(path);
                    ostd::io::println("'");
                }
            }
            Redirect::StdoutTo(path) => {
                // Non-echo stdout redirect: external capture deferred to Phase 17a.
                ostd::io::print("[redir > ");
                ostd::io::print(path);
                ostd::io::println("]");
            }
            Redirect::StdoutAppend(path) => {
                // Non-echo append redirect: same Phase 17a limitation.
                ostd::io::print("[redir >> ");
                ostd::io::print(path);
                ostd::io::println("]");
            }
            Redirect::StderrTo(path) => {
                ostd::io::print("[redir 2> ");
                ostd::io::print(path);
                ostd::io::println("]");
            }
        }
    }

    // Dispatch to shell built-ins first, then try to spawn from /bin/.
    let code = dispatch_builtin(prog, &args, jobs);
    // Set $? to the exit code so scripts can inspect it.
    set_var("?", i32_to_str(code));
    code
}

/// Match a case pattern against a value.
///
/// `*` is a catch-all; everything else is exact string equality.
fn case_matches(pattern: &str, value: &str) -> bool {
    pattern == "*" || pattern == value
}

/// Convert a small positional-arg index (1-9) to an owned `String` key.
///
/// Avoids `i32_to_str` which writes to a single shared static buffer —
/// calling it twice invalidates the first result while the second is alive.
fn usize_key(n: usize) -> String {
    let digit = b'0' + (n as u8).min(9);
    // SAFETY: `digit` is always a valid ASCII byte.
    String::from(unsafe { core::str::from_utf8_unchecked(core::slice::from_ref(&digit)) })
}

/// Convert a small non-negative integer to a &str backed by a fixed buffer.
///
/// Returns "0" for 0, the decimal string for 1-127, and "1" for anything else.
/// This avoids heap allocation for the `$?` variable.
fn i32_to_str(n: i32) -> &'static str {
    // Use a 'static lookup table for the most common exit codes (0–9).
    match n {
        0 => "0", 1 => "1", 2 => "2", 3 => "3", 4 => "4",
        5 => "5", 6 => "6", 7 => "7", 8 => "8", 9 => "9",
        127 => "127",
        _ => "1",
    }
}

/// Dispatch to the matching shell built-in.
///
/// Returns the exit code (0 = success, non-zero = error).
/// Falls through to `spawn_external` if no built-in matches.
fn dispatch_builtin(prog: &str, args: &[&str], jobs: &mut Jobs) -> i32 {
    let parts = core::iter::once(prog)
        .chain(args.iter().copied())
        .collect::<alloc::vec::Vec<_>>();
    let joined = parts.join(" ");
    let mut split = joined.split_whitespace();
    let _first = split.next();

    let result = match prog {
        // ── Filesystem ──────────────────────────────────────────────────
        "ls"    => crate::commands::cmd_ls(make_parts(args)),
        "cat"   => crate::commands::cmd_cat(make_parts(args)),
        "wc"    => crate::cmd_fs::cmd_wc(make_parts(args)),
        "head"  => crate::cmd_fs::cmd_head(make_parts(args)),
        "tail"  => crate::cmd_fs::cmd_tail(make_parts(args)),
        "grep"  => crate::cmd_fs::cmd_grep(make_parts(args)),
        "mkdir" => crate::cmd_fs::cmd_mkdir(make_parts(args)),
        "rmdir" => crate::cmd_fs::cmd_rmdir(make_parts(args)),
        "rm"    => crate::cmd_fs::cmd_rm(make_parts(args)),
        "vcat"    => crate::cmd_fs::cmd_vcat(make_parts(args)),
        "vwrite"  => crate::cmd_fs::cmd_vwrite(make_parts(args)),
        "vappend" => crate::cmd_fs::cmd_vappend(make_parts(args)),
        // ── System ──────────────────────────────────────────────────────
        "ps"     => crate::commands::cmd_ps(make_parts(args)),
        "pwd"    => crate::cmd_sys::cmd_pwd(make_parts(args)),
        "uname"  => crate::cmd_sys::cmd_uname(make_parts(args)),
        "free"   => crate::cmd_sys::cmd_free(make_parts(args)),
        "env"    => crate::cmd_sys::cmd_env(make_parts(args)),
        "uptime"   => crate::cmd_sys::cmd_uptime(make_parts(args)),
        "shutdown" => crate::cmd_sys::cmd_shutdown(),
        "sleep"    => crate::cmd_sys::cmd_sleep(make_parts(args)),
        "blktest"  => crate::cmd_sys::cmd_blkio_test(make_parts(args)),
        "echo"   => crate::commands::cmd_echo(make_parts(args)),
        "clear"  => crate::commands::cmd_clear(),
        "help"   => crate::commands::cmd_help(),
        "exec"   => crate::commands::cmd_exec(make_parts(args)),
        // ── Jobs ────────────────────────────────────────────────────────
        "jobs" => { print_jobs(jobs); Ok(()) }
        // ── Scripting ───────────────────────────────────────────────────
        // `.` is the POSIX short form of `source`.
        "source" | "." => cmd_source(args, jobs),
        // `test`/`[`: condition evaluation.  Returns Ok(()) (exit 0) on true,
        // Err (exit 1) on false.  `[` strips a trailing `]` argument.
        "test" => cmd_test(args),
        "[" => {
            let stripped: Vec<&str> = args.iter().copied()
                .filter(|&a| a != "]")
                .collect();
            cmd_test(&stripped)
        }
        "break"    => { set_loop_signal(LoopSignal::Break);    Ok(()) }
        "continue" => { set_loop_signal(LoopSignal::Continue); Ok(()) }
        "read" => cmd_read(args),
        "exit" => {
            let code = args.first().and_then(|s| {
                let mut n = 0i32;
                for ch in s.bytes() {
                    if !(b'0'..=b'9').contains(&ch) { return None; }
                    n = n.saturating_mul(10).saturating_add((ch - b'0') as i32);
                }
                Some(n)
            }).unwrap_or(0);
            request_exit(code);
            Ok(())
        }
        "unset" => {
            for name in args { unset_var(name); }
            Ok(())
        }
        // ── External / user-defined functions ───────────────────────────
        _ => {
            // Check the function table before trying to spawn an external binary.
            if let Some(body) = get_function(prog) {
                // Bind positional parameters $1..$9, $#, $@ for the function body.
                // Use owned Strings for index keys and saved values so we don't
                // alias the shared i32_to_str static buffer or VARS slot memory.
                let nargs = args.len().min(9);
                // Save keys "#" and "@" plus positional indices "1".."9".
                let mut saved: alloc::vec::Vec<(String, Option<String>)> =
                    alloc::vec::Vec::with_capacity(nargs + 2);
                for i in 1..=nargs {
                    let key = usize_key(i);
                    saved.push((key.clone(), get_var(&key).map(String::from)));
                }
                saved.push((String::from("#"), get_var("#").map(String::from)));
                saved.push((String::from("@"), get_var("@").map(String::from)));
                // Set new positional variables.
                for i in 1..=nargs {
                    set_var(&usize_key(i), args[i - 1]);
                }
                set_var("#", i32_to_str(nargs as i32));
                set_var("@", &args.join(" "));

                // Copy body to a local stack buffer so the 'static reference is
                // not held across the re-entrant parse+execute call.
                let mut buf = [0u8; 480];
                let bb = body.as_bytes();
                let blen = bb.len().min(479);
                buf[..blen].copy_from_slice(&bb[..blen]);
                let result = if let Ok(s) = core::str::from_utf8(&buf[..blen]) {
                    let ast = crate::parser::parse(s);
                    execute(&ast, jobs)
                } else { 1 };

                // Restore saved positional variables.
                for (k, v) in &saved {
                    match v {
                        Some(old) => set_var(k, old),
                        None      => unset_var(k),
                    }
                }
                return result;
            }
            return spawn_external(prog, args);
        }
    };
    match result { Ok(()) => 0, Err(_) => 1 }
}

/// Print all active jobs.
fn print_jobs(jobs: &Jobs) {
    for (id, state, name) in jobs.list() {
        ostd::io::print("[");
        ostd::io::print_usize(id);
        ostd::io::print("] ");
        ostd::io::print(match state {
            JobState::Running => "Running",
            JobState::Done    => "Done   ",
        });
        ostd::io::print("  ");
        ostd::io::println(name);
    }
}

/// `test` / `[` — evaluate a condition and return 0 (true) or 1 (false).
///
/// Supported forms:
/// - `-f path`   : file exists and is non-empty (vcat returns 0)
/// - `-z str`    : string is empty
/// - `-n str`    : string is non-empty
/// - `a = b`     : string equality
/// - `a != b`    : string inequality
fn cmd_test(args: &[&str]) -> ViResult<()> {
    let ok   = Ok(());
    let fail = Err(ViError::NotFound); // any non-Ok maps to exit code 1
    match args {
        ["-f", path] => {
            // File-existence check: vcat returns 0 if the file is present and
            // non-empty, 1 otherwise. Re-use the same VFS OP_READ path.
            let mut buf = [0u8; 8];
            if crate::cmd_fs::read_file_vfs(path, &mut buf) > 0 { ok } else { fail }
        }
        [s1, "-z"] | ["-z", s1] => if s1.is_empty() { ok } else { fail },
        [s1, "-n"] | ["-n", s1] => if !s1.is_empty() { ok } else { fail },
        _ => {
            // String comparison: `a = b` or `a != b`.
            // args may be ["a", "=", "b"] or ["a", "!=", "b"].
            if args.len() == 3 {
                let (a, op, b) = (args[0], args[1], args[2]);
                match op {
                    "=" | "==" => if a == b { ok } else { fail },
                    "!="       => if a != b { ok } else { fail },
                    _          => fail,
                }
            } else {
                fail
            }
        }
    }
}

/// `read [VAR]` — read one line from stdin (fd 0) into `$VAR` (default: `$REPLY`).
///
/// Blocks until a newline is received.  Uses `sys_read(0, ..)` — the same
/// mechanism as `AsyncStdin::read_line` minus the async/ANSI machinery.
fn cmd_read(args: &[&str]) -> ViResult<()> {
    let var = args.first().copied().unwrap_or("REPLY");
    let mut line = alloc::vec::Vec::<u8>::new();
    loop {
        let mut c = [0u8; 1];
        match ostd::syscall::sys_read(0, &mut c) {
            Ok(n) if n > 0 => {
                match c[0] {
                    b'\n' | b'\r' => break,
                    0x7F | 0x08 if !line.is_empty() => {
                        // Backspace — erase last char.
                        line.pop();
                        ostd::io::print("\x08 \x08");
                    }
                    b if line.len() < 127 => {
                        line.push(b);
                        // Echo the character so the user sees what they type.
                        if let Ok(s) = core::str::from_utf8(core::slice::from_ref(&b)) {
                            ostd::io::print(s);
                        }
                    }
                    _ => {}
                }
            }
            _ => { ostd::syscall::sys_yield(); }
        }
    }
    ostd::io::println(""); // newline after input
    if let Ok(s) = core::str::from_utf8(&line) {
        set_var(var, s);
    }
    Ok(())
}

/// `source <path>` — read a shell script from VFS and execute each line.
///
/// Lines starting with `#` and blank lines are skipped. The script runs in the
/// current shell's Jobs context, so spawns from the script are tracked normally.
/// Maximum script size is 4096 bytes (same limit as VFS OP_READ reply).
fn cmd_source(args: &[&str], jobs: &mut Jobs) -> ViResult<()> {
    let path = match args.first() {
        Some(p) => *p,
        None => {
            ostd::io::println("Usage: source <path>");
            return Ok(());
        }
    };
    let mut buf = alloc::vec![0u8; 4096];
    let n = crate::cmd_fs::read_file_vfs(path, &mut buf);
    if n == 0 {
        ostd::io::print("source: cannot open '");
        ostd::io::print(path);
        ostd::io::println("'");
        return Ok(());
    }
    let content = core::str::from_utf8(&buf[..n]).unwrap_or("");
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let ast = crate::parser::parse(line);
        execute(&ast, jobs);
    }
    Ok(())
}

/// Attempt to spawn an external binary from `/bin/<prog>`.
///
/// Arguments are published via `sys_set_spawn_args` (a reserved state-stash
/// slot) for the spawned cell to read on startup — `sys_spawn_from_path` does
/// not yet carry argv on the new cell's stack. We always set the slot (empty
/// when there are no args) so the cell never reads a previous command's args.
fn spawn_external(prog: &str, args: &[&str]) -> i32 {
    syscall::sys_set_spawn_args(&args.join(" "));

    let mut path = alloc::string::String::from("/bin/");
    path.push_str(prog);
    match syscall::sys_spawn_from_path(&path) {
        syscall::SyscallResult::Ok(_) => 0,
        syscall::SyscallResult::Err(_) => {
            ostd::io::print("shell: command not found: ");
            ostd::io::println(prog);
            127
        }
    }
}

/// Convert `args` into a `SplitWhitespace<'static>` for the existing `cmd_*` API.
///
/// Joins the slice with spaces, leaks the resulting `String` into a `'static`
/// reference, then splits on whitespace.  The leaked bytes (~arg length) are
/// bounded per command invocation and acceptable for a shell that runs until
/// reboot.
fn make_parts(args: &[&str]) -> core::str::SplitWhitespace<'static> {
    if args.is_empty() {
        return "".split_whitespace();
    }
    let joined = args.join(" ");
    // SAFETY: We intentionally leak the allocation so the returned SplitWhitespace
    // can carry a 'static lifetime.  The shell runs for the lifetime of the OS
    // session; per-invocation leaks are bounded by command argument sizes
    // (typically < 1 KB) and are acceptable.
    let leaked: &'static str = Box::leak(joined.into_boxed_str());
    leaked.split_whitespace()
}
