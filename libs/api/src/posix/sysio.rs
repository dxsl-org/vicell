// SPDX-License-Identifier: MPL-2.0
// Raw syscall ABI + POSIX syscall wrappers (_open, _read, _write, _exit, …)

#![allow(unsafe_code)]
#![allow(unused_variables)]

use core::ffi::{c_char, c_int, c_long, c_void};
use crate::syscall::ViSyscall;
use super::strings::strlen;

// ---------------------------------------------------------------------------
// Architecture-specific ecall/svc helper
// ---------------------------------------------------------------------------

#[cfg(target_arch = "riscv64")]
#[inline(always)]
pub(super) unsafe fn raw_syscall(id: ViSyscall, a0: usize, a1: usize, a2: usize, a3: usize) -> isize {
    let mut ret: isize;
    core::arch::asm!(
        "ecall",
        inlateout("a0") a0 => ret,
        in("a1") a1,
        in("a2") a2,
        in("a3") a3,
        in("a7") (id as usize),
        options(nostack, preserves_flags)
    );
    ret
}

// ARM64 ABI: x0=syscall_nr, x1=a0, x2=a1, x3=a2, x4=a3; ret in x0.
#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub(super) unsafe fn raw_syscall(id: ViSyscall, a0: usize, a1: usize, a2: usize, a3: usize) -> isize {
    let mut ret: isize;
    core::arch::asm!(
        "svc #0",
        inlateout("x0") id as usize => ret,
        in("x1") a0,
        in("x2") a1,
        in("x3") a2,
        in("x4") a3,
        options(nostack, preserves_flags)
    );
    ret
}

#[cfg(not(any(target_arch = "riscv64", target_arch = "aarch64")))]
pub(super) unsafe fn raw_syscall(_id: ViSyscall, _a0: usize, _a1: usize, _a2: usize, _a3: usize) -> isize {
    0
}

// ---------------------------------------------------------------------------
// C-compatible struct types
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct tms {
    pub tms_utime: c_long,
    pub tms_stime: c_long,
    pub tms_cutime: c_long,
    pub tms_cstime: c_long,
}

#[repr(C)]
pub struct stat {
    pub st_dev: c_int,
    pub st_ino: c_int,
    pub st_mode: c_int,
    pub st_nlink: c_int,
    pub st_uid: c_int,
    pub st_gid: c_int,
    pub st_rdev: c_int,
    pub st_size: c_long,
    pub st_atime: c_long,
    pub st_mtime: c_long,
    pub st_ctime: c_long,
    pub st_blksize: c_long,
    pub st_blocks: c_long,
}

#[repr(C)]
pub struct timeval {
    pub tv_sec: c_long,
    pub tv_usec: c_long,
}

// ---------------------------------------------------------------------------
// File / process stubs
// ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn _open(name: *const c_char, flags: c_int, mode: c_int) -> c_int {
    let len = strlen(name);
    raw_syscall(ViSyscall::Open, name as usize, len, flags as usize, mode as usize) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn _fcntl(_fd: c_int, _cmd: c_int, _arg: c_int) -> c_int { 0 }

#[no_mangle]
pub unsafe extern "C" fn _execve(_name: *const c_char, _argv: *const *const c_char, _env: *const *const c_char) -> c_int { -1 }

#[no_mangle]
pub unsafe extern "C" fn _fork() -> c_int { -1 }

#[no_mangle]
pub unsafe extern "C" fn _wait(_status: *mut c_int) -> c_int { -1 }

#[no_mangle]
pub unsafe extern "C" fn _times(buf: *mut tms) -> c_long {
    if !buf.is_null() {
        (*buf).tms_utime = 0;
        (*buf).tms_stime = 0;
        (*buf).tms_cutime = 0;
        (*buf).tms_cstime = 0;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn _link(_old: *const c_char, _new: *const c_char) -> c_int { -1 }

#[no_mangle]
pub unsafe extern "C" fn _unlink(_name: *const c_char) -> c_int { -1 }

#[no_mangle]
pub unsafe extern "C" fn _putchar(c: c_char) {
    let buf = [c as u8];
    raw_syscall(ViSyscall::Write, 1, buf.as_ptr() as usize, 1, 0);
}

// ---------------------------------------------------------------------------
// File I/O
// ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn _write(handle: c_int, buf: *const c_void, count: usize) -> c_int {
    raw_syscall(ViSyscall::Write, handle as usize, buf as usize, count, 0) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn _read(handle: c_int, buf: *mut c_void, count: usize) -> c_int {
    raw_syscall(ViSyscall::Read, handle as usize, buf as usize, count, 0) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn _lseek(handle: c_int, offset: c_long, whence: c_int) -> c_long {
    // Cast via isize to preserve sign on all platforms.
    raw_syscall(ViSyscall::Seek, handle as usize, offset as isize as usize, whence as usize, 0) as c_long
}

#[no_mangle]
pub unsafe extern "C" fn _fstat(handle: c_int, st: *mut stat) -> c_int {
    if !st.is_null() {
        core::ptr::write_bytes(st as *mut u8, 0, core::mem::size_of::<stat>());
        if handle <= 2 {
            (*st).st_mode = 0o20000 | 0o666; // S_IFCHR
        } else {
            (*st).st_mode = 0o100000 | 0o666; // S_IFREG
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn _isatty(handle: c_int) -> c_int {
    if handle >= 0 && handle <= 2 { 1 } else { 0 }
}

#[no_mangle]
pub unsafe extern "C" fn _getpid() -> c_int { 1 }

#[no_mangle]
pub unsafe extern "C" fn _kill(_pid: c_int, _sig: c_int) -> c_int { -1 }

#[no_mangle]
pub unsafe extern "C" fn _exit(status: c_int) -> ! {
    raw_syscall(ViSyscall::Exit, status as usize, 0, 0, 0);
    loop {}
}

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn _time(tloc: *mut c_long) -> c_long {
    let ret = raw_syscall(ViSyscall::GetTime, 3, 0, 0, 0); // op=3: epoch seconds
    let now = if ret >= 0 { ret as usize } else { 0 };
    if !tloc.is_null() { *tloc = now as c_long; }
    now as c_long
}

#[no_mangle]
pub unsafe extern "C" fn _gettimeofday(tv: *mut timeval, _tz: *mut c_void) -> c_int {
    if !tv.is_null() {
        let ret = raw_syscall(ViSyscall::GetTime, 3, 0, 0, 0);
        if ret >= 0 {
            (*tv).tv_sec = ret as c_long;
            (*tv).tv_usec = 0;
        }
    }
    0
}

// _sbrk returns NULL — Rust's GlobalAlloc owns the heap; no brk() in SAS.
#[no_mangle]
pub unsafe extern "C" fn _sbrk(_incr: c_int) -> *mut c_void {
    core::ptr::null_mut()
}
