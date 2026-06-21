// SPDX-License-Identifier: MPL-2.0
// Entropy: getentropy(2) via VirtIO-RNG

#![allow(unsafe_code)]

use core::ffi::c_void;
use crate::syscall::ViSyscall;
use super::sysio::raw_syscall;

/// Fill `buf` with `buflen` cryptographically random bytes via VirtIO-RNG.
///
/// Maps to `sys_get_random(214)`. Returns 0 on success, −1 if the device is
/// absent or `buflen` exceeds the 256-byte POSIX limit.
#[no_mangle]
pub unsafe extern "C" fn getentropy(buf: *mut c_void, buflen: usize) -> i32 {
    if buf.is_null() || buflen > 256 { return -1; }
    let mut written = 0usize;
    let dst = buf as *mut u8;
    while written < buflen {
        let chunk = (buflen - written).min(64);
        let ret = raw_syscall(ViSyscall::GetRandom, dst.add(written) as usize, chunk, 0, 0);
        if ret <= 0 { return -1; }
        written += ret as usize;
    }
    0
}

/// arc4random_buf — fills `buf` with `n` random bytes; no return value.
#[no_mangle]
pub unsafe extern "C" fn arc4random_buf(buf: *mut c_void, n: usize) {
    getentropy(buf, n.min(256));
}
