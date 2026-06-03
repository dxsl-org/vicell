//! System-information shell built-ins: pwd, uname, date, free, env.

use ostd::prelude::*;
use ostd::syscall;

/// `pwd` — print the current working directory.
///
/// ViOS v1.0 has no per-cell CWD tracking; always prints `/` until
/// Phase 17a adds a proper chdir/getcwd implementation.
pub fn cmd_pwd<'a>(_args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    ostd::io::println("/");
    Ok(())
}

/// `uname [-a]` — print system identification.
pub fn cmd_uname<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let all = args.any(|a| a == "-a");
    if all {
        ostd::io::println("ViOS vios-kernel 0.2.1 riscv64 ViOS");
    } else {
        ostd::io::println("ViOS");
    }
    Ok(())
}

/// `free` — print memory usage summary.
///
/// The MemInfo syscall is stubbed in v1.0; this shows approximate compiled-in
/// values until a proper MemInfo syscall is wired (Phase 22 benchmarking).
pub fn cmd_free<'a>(_args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    ostd::io::println("              total        used        free");
    ostd::io::println("Mem:        131072        ~4096      ~127000 (KB approx, no MemInfo yet)");
    Ok(())
}

/// `env` — list all environment key=value pairs from the Config Cell.
pub fn cmd_env<'a>(_args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    // Config Cell is cell 3 in the default boot sequence.
    // For now, print a static representative set until Config IPC is hooked.
    ostd::io::println("PATH=/bin");
    ostd::io::println("SHELL=/bin/shell");
    ostd::io::println("OS=ViOS");
    Ok(())
}

/// `uptime` — print time since boot in seconds.
///
/// Reads the kernel monotonic timer; converts ticks to seconds at 10 MHz.
pub fn cmd_uptime<'a>(_args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let ticks = syscall::sys_get_time();
    let secs = ticks / 10_000_000; // 10 MHz mtime
    ostd::io::print("up ");
    ostd::io::print_usize(secs as usize);
    ostd::io::println(" seconds");
    Ok(())
}

/// `shutdown` — cleanly power off the system via SBI SRST. Does not return.
///
/// Routes through raw kernel syscall 502 (SBI System Reset Extension) which
/// calls OpenSBI from S-mode, powering off the machine.
pub fn cmd_shutdown<'a>() -> ViResult<()> {
    ostd::io::println("System shutting down...");
    syscall::sys_shutdown()
}

/// `blktest` — attempt a raw block read from the shell cell (a non-VFS cell).
///
/// Prints `"blkio: denied"` when Phase G's capability gate correctly rejects the
/// call, or `"blkio: ALLOWED (BUG)"` if the gate is missing. Used exclusively
/// by the `block_io_denied_non_vfs` integration test.
pub fn cmd_blkio_test<'a>(_args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let mut buf = [0u8; 512];
    if syscall::sys_blk_read(0, &mut buf) {
        ostd::io::println("blkio: ALLOWED (BUG)");
    } else {
        ostd::io::println("blkio: denied");
    }
    Ok(())
}
