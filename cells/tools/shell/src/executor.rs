//! Shell AST executor — runs parsed commands, handles pipes and redirects.
//!
//! Pipes between built-in commands are implemented via an in-memory `OutputSink`:
//! each pipeline stage's output is captured into a `Vec<u8>`, then passed as
//! stdin to the next stage.  `SinkGuard` (RAII, Law 8) ensures the sink is
//! restored on every exit path, including early returns.

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use crate::parser::{Ast, Cmd, Redirect};
use crate::jobs::{Jobs, JobState};
use ostd::prelude::*;
use ostd::syscall;

// ── Output sink (pipeline capture) ───────────────────────────────────────────
//
// All shell command output MUST go through `shell_print` / `shell_println`
// rather than `ostd::io::print` directly.  When the sink is set to `Buffer`,
// output is captured into the pointed-to Vec instead of the serial console.
//
// Only the single shell task reads or writes `CURRENT_SINK`.  External cells
// never call `shell_print`, so there is no concurrent-access hazard.
// `CURRENT_STDIN` follows the same pattern for pipe-fed stdin data.

enum OutputSink { Console, Buffer(*mut Vec<u8>) }

/// Newtype that asserts `Sync` for an `UnsafeCell` in a single-task context.
///
/// Only valid when ALL accesses are guaranteed to come from one task (the shell).
/// External cells never call `shell_print` or `shell_stdin`, so the invariant holds.
struct SingleTaskCell<T>(UnsafeCell<T>);

// SAFETY: the shell is a single-task executor; no other task accesses these statics.
unsafe impl<T> Sync for SingleTaskCell<T> {}

impl<T> SingleTaskCell<T> {
    const fn new(val: T) -> Self { Self(UnsafeCell::new(val)) }
    fn get(&self) -> *mut T { self.0.get() }
}

static CURRENT_SINK: SingleTaskCell<OutputSink> = SingleTaskCell::new(OutputSink::Console);

/// Points to the current stdin buffer for pipe-aware built-ins.
/// Null when no pipe is active (reads from real serial stdin).
static CURRENT_STDIN: SingleTaskCell<*const [u8]> =
    SingleTaskCell::new(core::ptr::null::<[u8; 0]>() as *const [u8]);

/// RAII guard that restores the previous `OutputSink` on all exit paths (Law 8).
struct SinkGuard(OutputSink);

impl SinkGuard {
    fn new(new_sink: OutputSink) -> Self {
        // SAFETY: single shell task; exclusive access guaranteed.
        let prev = unsafe { core::mem::replace(&mut *CURRENT_SINK.get(), new_sink) };
        SinkGuard(prev)
    }
}

impl Drop for SinkGuard {
    fn drop(&mut self) {
        // SAFETY: single shell task; restoring saved sink on any exit path.
        unsafe { *CURRENT_SINK.get() = core::mem::replace(&mut self.0, OutputSink::Console); }
    }
}

/// Route command output through the current sink.
///
/// All built-in output calls this instead of `ostd::io::print` so pipeline
/// capture works.  The prompt and internal error diagnostics call
/// `ostd::io::print` directly to always reach the console regardless of sink.
pub fn shell_print(s: &str) {
    // SAFETY: only the shell task reads/writes CURRENT_SINK.
    match unsafe { &*CURRENT_SINK.get() } {
        OutputSink::Console   => ostd::io::print(s),
        OutputSink::Buffer(v) => unsafe { (**v).extend_from_slice(s.as_bytes()) },
    }
}

/// `shell_print(s)` followed by a newline.
pub fn shell_println(s: &str) { shell_print(s); shell_print("\n"); }

/// Return the current pipe-fed stdin bytes, or an empty slice.
///
/// Commands that accept either a file argument or stdin (e.g., `grep`, `wc`)
/// call this when no file path is given.
pub fn shell_stdin() -> &'static [u8] {
    // SAFETY: CURRENT_STDIN is set and live for the duration of dispatch_builtin.
    let ptr = unsafe { *CURRENT_STDIN.get() };
    if ptr.is_null() { &[] } else { unsafe { &*ptr } }
}

/// All recognized shell built-in names, used by tab completion.
pub const BUILTINS: &[&str] = &[
    "alias", "awk", "bg", "blktest", "break", "cat", "clear", "continue", "echo", "env",
    "exec", "exit", "export", "fg", "find", "free", "grep", "head", "help", "jobs",
    "kill", "ls", "mkdir", "ps", "pwd", "read", "rm", "rmdir", "sed", "shutdown",
    "sleep", "snapshot", "sort", "source", "tail", "tee", "test", "top", "unalias",
    "uniq", "unset", "uname", "uptime", "vappend", "vcat", "vwrite", "wc",
];

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
/// Capture the output of a built-in command without printing it.
///
/// Only a small capturable set is supported: `echo`, `vcat`/`cat`, `pwd`.
/// External binaries and unsupported built-ins return an empty String.
/// Nested `$(...)` is the caller's responsibility to reject before calling.
fn run_capture(inner: &str) -> String {
    let mut words = inner.split_whitespace();
    let cmd = match words.next() { Some(c) => c, None => return String::new() };
    let args: alloc::vec::Vec<&str> = words.collect();
    match cmd {
        "echo" => {
            let bytes = crate::commands::cmd_echo_to_vec(&args);
            String::from(core::str::from_utf8(&bytes).unwrap_or(""))
        }
        "vcat" | "cat" => {
            if let Some(path) = args.first() {
                let mut buf = [0u8; 480];
                let n = crate::cmd_fs::read_file_vfs(path, &mut buf);
                if n > 0 {
                    String::from(core::str::from_utf8(&buf[..n]).unwrap_or(""))
                } else { String::new() }
            } else { String::new() }
        }
        "pwd" => String::from("/\n"),
        _ => String::new(),
    }
}

fn expand_token(s: &str) -> String {
    if !s.contains('$') { return String::from(s); }
    let mut result = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            if next == b'(' {
                // Command substitution: $(...). Scan to matching ')'.
                // Single-level only — nested $( $() ) passes through as literal.
                let inner_start = i + 2;
                let mut depth = 1usize;
                let mut j = inner_start;
                while j < bytes.len() {
                    if bytes[j] == b'(' { depth += 1; }
                    else if bytes[j] == b')' {
                        depth -= 1;
                        if depth == 0 { break; }
                    }
                    j += 1;
                }
                if depth == 0 {
                    // SAFETY: bytes[inner_start..j] is ASCII shell token chars.
                    let inner = unsafe { core::str::from_utf8_unchecked(&bytes[inner_start..j]) };
                    // Reject nested $(...) — pass $(  literally so the user can see the issue.
                    if !inner.contains("$(") {
                        let captured = run_capture(inner.trim());
                        result.push_str(captured.trim_end_matches('\n'));
                        i = j + 1;
                        continue;
                    }
                }
                // Unmatched paren or nested: emit '$(' literally and continue.
                result.push('$');
                i += 1;
                continue;
            }
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

/// Parse and execute `line`, capturing all `shell_print` output into a `Vec<u8>`.
///
/// Used by the `shell_test` feature harness to assert on command output without
/// requiring a real serial console.  The `SinkGuard` ensures the sink is restored
/// even if the command panics or returns early.
///
/// Precondition: must be called from the single shell task (same `SingleTaskCell`
/// invariant as `shell_print`).
#[cfg(feature = "shell_test")]
pub fn capture_line(line: &str, jobs: &mut Jobs) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let _guard = SinkGuard::new(OutputSink::Buffer(&mut out as *mut _));
    let ast = crate::parser::parse(line);
    execute(&ast, jobs);
    drop(_guard);
    out
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
            // Cooperative background: the shell is a single-task executor with no
            // async spawn capability for built-ins. `cmd &` runs synchronously and
            // is marked Done before control returns. True async background would
            // require spawning the command as a separate Cell via SpawnCap — not
            // in scope for G1. `fg`/`bg` built-ins report this limitation.
            let name = cmd.argv.first().map(String::as_str).unwrap_or("?");
            let jid = jobs.add(name);
            // Background job notification always goes to console, not the sink.
            ostd::io::print("["); ostd::io::print_usize(jid); ostd::io::println("] running");
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
///
/// Intermediate stages are captured into `Vec<u8>` buffers; the final stage
/// runs directly through the current sink so its exit code is preserved and
/// any outer capture (nested pipeline or `$(...)`) captures it correctly.
fn exec_pipeline(cmds: &[Cmd], jobs: &mut Jobs) -> i32 {
    if cmds.is_empty() { return 0; }
    let last_idx = cmds.len() - 1;
    let mut stdin_data: Vec<u8> = Vec::new();

    for (i, cmd) in cmds.iter().enumerate() {
        if i == last_idx {
            // Last stage: run directly (no intermediate capture).
            // Wire pipe stdin so built-ins without a file path read from it.
            // SAFETY: stdin_data is alive for the duration of exec_cmd.
            unsafe { *CURRENT_STDIN.get() = stdin_data.as_slice() as *const [u8]; }
            let code = exec_cmd(cmd, &stdin_data, jobs);
            unsafe { *CURRENT_STDIN.get() = core::ptr::null::<[u8; 0]>() as *const [u8]; }
            return code;
        }
        stdin_data = capture_cmd(cmd, &stdin_data, jobs);
    }
    0
}

/// Run a command and capture its output into a `Vec<u8>`.
///
/// Uses `OutputSink::Buffer` so that any built-in calling `shell_print` writes
/// into `out` instead of the serial console.  The `SinkGuard` ensures the
/// previous sink (Console or an outer Buffer for nested pipelines) is restored
/// on every exit path.
fn capture_cmd(cmd: &Cmd, stdin: &[u8], jobs: &mut Jobs) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let _guard = SinkGuard::new(OutputSink::Buffer(&mut out as *mut _));
    exec_cmd(cmd, stdin, jobs);
    // _guard.drop() restores the previous sink before `out` is returned.
    drop(_guard);
    out
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

    // echo with stdout redirect: fast path using cmd_echo_to_vec (no OutputSink needed).
    if prog == "echo" {
        if let Some(Redirect::StdoutTo(path)) =
            cmd.redirects.iter().find(|r| matches!(r, Redirect::StdoutTo(_)))
        {
            let bytes = crate::commands::cmd_echo_to_vec(&args);
            if !crate::cmd_fs::write_file(path, &bytes) {
                ostd::io::print("echo: cannot write '"); ostd::io::print(path); ostd::io::println("'");
            }
            return 0;
        }
        if let Some(Redirect::StdoutAppend(path)) =
            cmd.redirects.iter().find(|r| matches!(r, Redirect::StdoutAppend(_)))
        {
            let bytes = crate::commands::cmd_echo_to_vec(&args);
            if !crate::cmd_fs::append_file(path, &bytes) {
                ostd::io::print("echo: cannot append '"); ostd::io::print(path); ostd::io::println("'");
            }
            return 0;
        }
    }

    // StdinFrom redirect: preload the file into a buffer and expose it via
    // shell_stdin() so built-ins (grep, wc, …) can read from it.
    let stdin_file_buf: Vec<u8>;
    let effective_stdin: &[u8] = if let Some(Redirect::StdinFrom(path)) =
        cmd.redirects.iter().find(|r| matches!(r, Redirect::StdinFrom(_)))
    {
        stdin_file_buf = {
            let mut buf = alloc::vec![0u8; 4096];
            let n = crate::cmd_fs::read_file_vfs(path, &mut buf);
            if n == 0 {
                ostd::io::print("shell: cannot open '"); ostd::io::print(path); ostd::io::println("'");
            }
            buf[..n].to_vec()
        };
        &stdin_file_buf
    } else {
        _stdin
    };

    // Detect stdout/stderr redirect for non-echo commands.
    //
    // ViCell has one output channel (serial console). `2>file` is therefore
    // semantically equivalent to `>file` — both capture `shell_print` output.
    // When both `>` and `2>` are present, `>` takes precedence; `2>` is a
    // documented no-op in that case (single-channel limitation).
    let stdout_redir = cmd.redirects.iter().find_map(|r| match r {
        Redirect::StdoutTo(path)     => Some((path.clone(), false)),
        Redirect::StdoutAppend(path) => Some((path.clone(), true)),
        _ => None,
    }).or_else(|| cmd.redirects.iter().find_map(|r| match r {
        // Fallback: StderrTo reuses the stdout-capture path (one-channel shell).
        Redirect::StderrTo(path) => Some((path.clone(), false)),
        _ => None,
    }));

    // Wire the pipe-fed stdin so pipe-aware built-ins can read it.
    // SAFETY: effective_stdin is alive for the duration of dispatch_builtin.
    unsafe { *CURRENT_STDIN.get() = effective_stdin as *const [u8]; }

    let code = if let Some((path, append)) = stdout_redir {
        // Capture this command's output into a buffer, then write to VFS.
        let mut captured: Vec<u8> = Vec::new();
        {
            let _guard = SinkGuard::new(OutputSink::Buffer(&mut captured as *mut _));
            dispatch_builtin(prog, &args, jobs);
        } // _guard drops here, restoring sink before the VFS write
        crate::cmd_fs::vfs_write_chunked(&path, &captured, append);
        0
    } else {
        dispatch_builtin(prog, &args, jobs)
    };

    // Clear stdin pointer; keep CURRENT_SINK unmodified (exec_cmd doesn't own it).
    unsafe { *CURRENT_STDIN.get() = core::ptr::null::<[u8; 0]>() as *const [u8]; }

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
        "find"  => crate::cmd_fs::cmd_find(make_parts(args)),
        "uniq"  => crate::cmd_fs::cmd_uniq(make_parts(args)),
        "sort"  => crate::cmd_fs::cmd_sort(make_parts(args)),
        "tee"   => crate::cmd_fs::cmd_tee(make_parts(args)),
        "sed"   => crate::cmd_fs::cmd_sed(make_parts(args)),
        "mkdir" => crate::cmd_fs::cmd_mkdir(make_parts(args)),
        "rmdir" => crate::cmd_fs::cmd_rmdir(make_parts(args)),
        "rm"    => crate::cmd_fs::cmd_rm(make_parts(args)),
        "vcat"    => crate::cmd_fs::cmd_vcat(make_parts(args)),
        "vwrite"  => crate::cmd_fs::cmd_vwrite(make_parts(args)),
        "vappend" => crate::cmd_fs::cmd_vappend(make_parts(args)),
        "awk"  => crate::cmd_fs::cmd_awk(make_parts(args)),
        "top"  => crate::commands::cmd_top(make_parts(args)),
        "kill" => crate::commands::cmd_kill(make_parts(args)),
        // ── Snapshot ────────────────────────────────────────────────────
        "snapshot" => {
            shell_println("[shell] writing warm-boot snapshot...");
            match ostd::syscall::sys_snapshot() {
                ostd::syscall::SyscallResult::Ok(n) if n > 0 => {
                    shell_print(&alloc::format!("[shell] snapshot: wrote {} frames. Reboot for warm boot.\n", n));
                    Ok(())
                }
                _ => { shell_println("[shell] snapshot: failed"); Err(ViError::Unknown) }
            }
        }
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
        // fg/bg: ViCell background jobs run synchronously (cooperative scheduler,
        // single-task shell). All &-jobs already completed before fg/bg is called.
        "fg" | "bg" => {
            shell_println("fg/bg: no job control — background jobs run synchronously in this shell");
            Ok(())
        }
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
        shell_print(&alloc::format!("[{}] {}  {}\n", id,
            match state { JobState::Running => "Running", JobState::Done => "Done   " },
            name));
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
        syscall::SyscallResult::Ok(tid) => {
            // Foreground: block until the child exits so it owns the console
            // (stdin/UART). Without this the shell loops back to read the next
            // line and races interactive children (e.g. `hypha`) for keystrokes.
            // Fast commands return immediately (kernel Wait short-circuits when
            // the child is already Terminated). Background (`&`) already runs
            // synchronously in G1, so this does not regress it.
            match syscall::sys_wait(tid) {
                syscall::SyscallResult::Ok(code) => code as i32,
                syscall::SyscallResult::Err(_) => 0,
            }
        }
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
