//! Supervisor Binary Interface (SBI) wrappers
//!
//! Allows S-mode kernel to make requests to M-mode OpenSBI firmware.

#![allow(dead_code)]

// SBI Extension IDs
const SBI_EID_TIMER: usize = 0x54494D45;
const SBI_EID_LEGACY_SET_TIMER: usize = 0x00;
const SBI_EID_DBCN: usize = 0x4442434E;
const SBI_FID_DBCN_WRITE: usize = 0;

#[inline(always)]
fn sbi_call(eid: usize, fid: usize, arg0: usize, arg1: usize, arg2: usize) -> (usize, usize) {
    let mut error;
    let mut value;
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a7") eid,
            in("a6") fid,
            in("a0") arg0,
            in("a1") arg1,
            in("a2") arg2,
            lateout("a0") error,
            lateout("a1") value,
            options(nostack)
        );
    }
    (error, value)
}

/// Write a character to debug console (DBCN)
pub fn console_putchar(c: u8) -> Result<(), usize> {
    let byte = c as u8;
    let ptr = &byte as *const u8 as usize;
    // console_write(num_bytes=1, base_addr_lo=ptr, base_addr_hi=0)
    let (error, _value) = sbi_call(SBI_EID_DBCN, SBI_FID_DBCN_WRITE, 1, ptr, 0);

    if error == 0 {
        Ok(())
    } else {
        Err(error)
    }
}

const SBI_FID_DBCN_READ: usize = 1;

/// Read a character from debug console (DBCN)
pub fn console_getchar() -> isize {
    let mut byte: u8 = 0;
    let ptr = &mut byte as *mut u8 as usize;
    let (error, value) = sbi_call(SBI_EID_DBCN, SBI_FID_DBCN_READ, 1, ptr, 0);

    if error == 0 && value == 1 {
        byte as isize
    } else {
        -1
    }
}

const SBI_FID_SET_TIMER: usize = 0;

/// Set timer
pub fn set_timer(stime_value: u64) {
    #[cfg(target_arch = "riscv64")]
    sbi_call(SBI_EID_TIMER, SBI_FID_SET_TIMER, stime_value as usize, 0, 0);
}

// SBI System Reset (SRST) extension — used by the panic handler to reboot a
// dead kernel instead of freezing (a robot must come back up, not brick).
const SBI_EID_SRST: usize = 0x53525354;
const SBI_FID_SYSTEM_RESET: usize = 0;
/// SRST reset_type: graceful power-off.
pub const SBI_RESET_SHUTDOWN: usize = 0x0;
/// SRST reset_type: full re-init (re-runs firmware + kernel from scratch).
pub const SBI_RESET_COLD_REBOOT: usize = 0x1;

/// Request a system reset via SBI SRST.
///
/// On success the machine resets and this never returns; it returns only if the
/// firmware does not implement SRST, letting the caller fall back to a halt loop.
pub fn system_reset(reset_type: usize, reset_reason: usize) {
    sbi_call(SBI_EID_SRST, SBI_FID_SYSTEM_RESET, reset_type, reset_reason, 0);
}
