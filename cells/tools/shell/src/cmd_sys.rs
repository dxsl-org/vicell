//! System-information shell built-ins: pwd, uname, date, free, env.

use ostd::prelude::*;
use ostd::syscall;

/// `pwd` — print the current working directory.
///
/// ViCell v1.0 has no per-cell CWD tracking; always prints `/` until
/// Phase 17a adds a proper chdir/getcwd implementation.
pub fn cmd_pwd<'a>(_args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    crate::executor::shell_println("/");
    Ok(())
}

/// `uname [-a]` — print system identification.
pub fn cmd_uname<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let all = args.any(|a| a == "-a");
    crate::executor::shell_println(if all { "ViCell vicell-kernel 0.2.1 riscv64 ViCell" } else { "ViCell" });
    Ok(())
}

/// `free` — print memory usage summary.
///
/// The MemInfo syscall is stubbed in v1.0; this shows approximate compiled-in
/// values until a proper MemInfo syscall is wired.
pub fn cmd_free<'a>(_args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    crate::executor::shell_println("              total        used        free");
    crate::executor::shell_println("Mem:        131072        ~4096      ~127000 (KB approx, no MemInfo yet)");
    Ok(())
}

/// `env` — list all environment key=value pairs from the Config Cell.
pub fn cmd_env<'a>(_args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    crate::executor::shell_println("PATH=/bin");
    crate::executor::shell_println("SHELL=/bin/shell");
    crate::executor::shell_println("OS=ViCell");
    Ok(())
}

/// `uptime` — print time since boot in seconds.
///
/// Reads the kernel monotonic timer; converts ticks to seconds at 10 MHz.
pub fn cmd_uptime<'a>(_args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let ticks = syscall::sys_get_time();
    let secs = ticks / 10_000_000; // 10 MHz mtime
    crate::executor::shell_print(&alloc::format!("up {} seconds\n", secs));
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

/// `sleep <seconds>` — pause execution for the given number of seconds.
///
/// Uses the kernel monotonic timer (mtime at 10 MHz on QEMU RV64).
/// Yields on each iteration so other tasks keep running during the delay.
pub fn cmd_sleep<'a>(mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    const TIMER_HZ: u64 = 10_000_000; // 10 MHz mtime
    let secs: u64 = match args.next().and_then(|s| {
        let mut n = 0u64;
        for ch in s.bytes() {
            if ch < b'0' || ch > b'9' { return None; }
            n = n.saturating_mul(10).saturating_add((ch - b'0') as u64);
        }
        Some(n)
    }) {
        Some(n) => n,
        None => {
            ostd::io::println("Usage: sleep <seconds>");
            return Ok(());
        }
    };
    let deadline = syscall::sys_get_time().saturating_add(secs.saturating_mul(TIMER_HZ));
    while syscall::sys_get_time() < deadline {
        ostd::task::yield_now();
    }
    Ok(())
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
