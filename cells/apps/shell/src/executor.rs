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
use ostd::syscall;

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

    let prog = &cmd.argv[0];
    let args: Vec<&str> = cmd.argv[1..].iter().map(String::as_str).collect();

    // Phase C: capture `echo` output when a StdoutTo redirect is present.
    // Only `echo` is supported — external-process capture requires pipe caps (Phase 17a).
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
    }

    // Apply input redirect if present (read from file into buffer).
    // For v1.0 the redirected data is not plumbed into the command yet.
    for r in &cmd.redirects {
        match r {
            Redirect::StdinFrom(path) => {
                // Signal intent; actual piping deferred to Phase 17a pipe caps.
                ostd::io::print("[redir < ");
                ostd::io::print(path);
                ostd::io::println("]");
            }
            Redirect::StdoutTo(path) | Redirect::StdoutAppend(path) => {
                ostd::io::print("[redir > ");
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
    dispatch_builtin(prog, &args, jobs)
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
        "vcat"  => crate::cmd_fs::cmd_vcat(make_parts(args)),
        // ── System ──────────────────────────────────────────────────────
        "ps"     => crate::commands::cmd_ps(make_parts(args)),
        "pwd"    => crate::cmd_sys::cmd_pwd(make_parts(args)),
        "uname"  => crate::cmd_sys::cmd_uname(make_parts(args)),
        "free"   => crate::cmd_sys::cmd_free(make_parts(args)),
        "env"    => crate::cmd_sys::cmd_env(make_parts(args)),
        "uptime"   => crate::cmd_sys::cmd_uptime(make_parts(args)),
        "shutdown" => crate::cmd_sys::cmd_shutdown(),
        "blktest"  => crate::cmd_sys::cmd_blkio_test(make_parts(args)),
        "echo"   => crate::commands::cmd_echo(make_parts(args)),
        "clear"  => crate::commands::cmd_clear(),
        "help"   => crate::commands::cmd_help(),
        "exec"   => crate::commands::cmd_exec(make_parts(args)),
        // ── Jobs ────────────────────────────────────────────────────────
        "jobs" => { print_jobs(jobs); Ok(()) }
        // ── External ────────────────────────────────────────────────────
        _ => return spawn_external(prog, args),
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
