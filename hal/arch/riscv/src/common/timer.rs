//! Timer Interface (S-mode)
//!
//! Provides access to the RISC-V timer via SBI calls and the `time` CSR.
//! Direct CLINT access is NOT available in S-mode — use SBI for `mtimecmp`.

/// Ticks per 10 ms at the assumed 10 MHz `mtime` clock on QEMU virt.
///
/// Used to set the preemptive timeslice duration.  If the actual clock
/// differs (detectable via DTB), callers should adjust accordingly.
pub const TICKS_PER_10MS: u64 = 100_000;

/// Read the current machine time (via 'time' CSR)
pub fn read_mtime() -> u64 {
    let time: u64;
    #[cfg(target_arch = "riscv64")]
    unsafe {
        // Read "time" CSR (0xC01) which mirrors mtime
        core::arch::asm!("csrr {0}, time", out(reg) time);
    }
    #[cfg(not(target_arch = "riscv64"))]
    {
        time = 0;
    }

    time
}

/// Get time in milliseconds (assuming 10MHz clock)
pub fn time_ms() -> u64 {
    read_mtime() / 10_000
}

/// Set a timer interrupt to fire after `ms` milliseconds
pub fn set_timer_ms(ms: u64) {
    let current = read_mtime();
    let target = current + (ms * 10_000);

    // Use SBI call to set timer in M-mode
    crate::common::sbi::set_timer(target);
}
