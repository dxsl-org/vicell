//! C-callable bridge from MicroPython's C world to ViOS syscalls.
//!
//! `modvnet.c` declares `extern` prototypes for these symbols and calls them
//! directly.  All functions are safe to call from C as long as the buffer
//! invariants hold (documented per function).

use ostd::syscall::{sys_recv, sys_send, sys_try_recv, SyscallResult};

/// Send `len` bytes from `buf` to the cell at `endpoint`.
///
/// # Safety
/// `buf` must be valid for `len` bytes for the duration of this call.
#[no_mangle]
pub unsafe extern "C" fn vios_net_send(endpoint: usize, buf: *const u8, len: usize) {
    // SAFETY: caller guarantees buf[0..len] is valid and live.
    let slice = unsafe { core::slice::from_raw_parts(buf, len) };
    sys_send(endpoint, slice);
}

/// Blocking receive into `buf[0..buf_len]`. Returns sender task ID on success,
/// -1 on error. Clears buf before receive so zero-scan works correctly.
///
/// # Safety
/// `buf` must be valid for `buf_len` bytes, uniquely accessible for the
/// duration of this call.
#[no_mangle]
pub unsafe extern "C" fn vios_net_recv(from_id: usize, buf: *mut u8, buf_len: usize) -> isize {
    // SAFETY: caller guarantees buf[0..buf_len] is valid and uniquely accessible.
    let slice = unsafe { core::slice::from_raw_parts_mut(buf, buf_len) };
    slice.fill(0);
    match sys_recv(from_id, slice) {
        SyscallResult::Ok(sender) => sender as isize,
        _ => -1,
    }
}

/// Non-blocking receive. Returns sender task ID if a message was available,
/// -1 if the queue was empty or an error occurred.
///
/// # Safety
/// `buf` must be valid for `buf_len` bytes, uniquely accessible for the
/// duration of this call.
#[no_mangle]
pub unsafe extern "C" fn vios_net_try_recv(from_id: usize, buf: *mut u8, buf_len: usize) -> isize {
    // SAFETY: caller guarantees buf[0..buf_len] is valid and uniquely accessible.
    let slice = unsafe { core::slice::from_raw_parts_mut(buf, buf_len) };
    slice.fill(0);
    match sys_try_recv(from_id, slice) {
        SyscallResult::Ok(sender) => sender as isize,
        _ => -1,
    }
}

/// Yield the current task's time slice (cooperative scheduling).
#[no_mangle]
pub extern "C" fn vios_net_yield() {
    ostd::task::yield_now();
}
