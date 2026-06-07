// SPDX-License-Identifier: MPL-2.0

//! POSIX Shim Layer (The Bridge)
//!
//! This module provides a minimal implementation of memory management 
//! and low-level syscalls to support Newlib (libc).

#![allow(unsafe_code)]
#![allow(unused_variables)]
#![allow(non_upper_case_globals)]
#![cfg(any(target_arch = "riscv64", target_arch = "wasm32", doc))]

extern crate alloc;

use crate::ipc::{decode, encode, IPC_BUF_SIZE, NetRequest, NetResponse};
use crate::syscall::ViSyscall;
use core::alloc::Layout;
use core::ffi::{c_char, c_int, c_long, c_void};
use core::ptr;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

// ----------------------------------------------------------------------------
// 1. Memory Management (Overrides libc weak symbols)
// ----------------------------------------------------------------------------

extern "Rust" {
    // We assume the binary linking this (ostd/app) has defined a global allocator.
}

// Internal Header Strategy
#[repr(C)]
struct AllocHeader {
    size: usize,
    magic: usize,
}
const HEADER_MAGIC: usize = 0xDEADC0DE;
const HEADER_SIZE: usize = core::mem::size_of::<AllocHeader>();
const ALIGN: usize = 16;

#[no_mangle]
pub unsafe extern "C" fn malloc(size: usize) -> *mut c_void {
    malloc_impl(size)
}

#[no_mangle]
pub unsafe extern "C" fn free(ptr: *mut c_void) {
    free_impl(ptr)
}

#[no_mangle]
pub unsafe extern "C" fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    realloc_impl(ptr, size)
}

#[no_mangle]
pub unsafe extern "C" fn calloc(nmemb: usize, size: usize) -> *mut c_void {
    let total_size = match nmemb.checked_mul(size) {
        Some(s) => s,
        None => return ptr::null_mut(),
    };
    let ptr = malloc(total_size);
    if !ptr.is_null() {
        // Use internal memset to avoid dependency loops if libc absent?
        // We will provide memset below.
        memset(ptr, 0, total_size);
    }
    ptr
}

unsafe fn malloc_impl(size: usize) -> *mut c_void {
    let total_size = match size.checked_add(HEADER_SIZE) {
        Some(s) => s,
        None => return ptr::null_mut(),
    };

    let layout = match Layout::from_size_align(total_size, ALIGN) {
        Ok(l) => l,
        Err(_) => return ptr::null_mut(),
    };

    let ptr = alloc::alloc::alloc(layout);
    if ptr.is_null() {
        return ptr::null_mut();
    }

    let header = ptr as *mut AllocHeader;
    (*header).size = size;
    (*header).magic = HEADER_MAGIC;

    let res = ptr.add(HEADER_SIZE) as *mut c_void;
    res
}

unsafe fn free_impl(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }

    let real_ptr = (ptr as *mut u8).sub(HEADER_SIZE);
    let header = real_ptr as *mut AllocHeader;

    if (*header).magic != HEADER_MAGIC {
        return;
    }

    let size = (*header).size;
    let total_size = size + HEADER_SIZE;
    let layout = Layout::from_size_align_unchecked(total_size, ALIGN);

    alloc::alloc::dealloc(real_ptr, layout);
}

unsafe fn realloc_impl(ptr: *mut c_void, new_size: usize) -> *mut c_void {
    if ptr.is_null() {
        return malloc_impl(new_size);
    }
    if new_size == 0 {
        free_impl(ptr);
        return ptr::null_mut();
    }

    let real_ptr = (ptr as *mut u8).sub(HEADER_SIZE);
    let header = real_ptr as *mut AllocHeader;

    if (*header).magic != HEADER_MAGIC {
        return ptr::null_mut();
    }

    let old_size = (*header).size;
    let total_old_size = old_size + HEADER_SIZE;
    let old_layout = Layout::from_size_align_unchecked(total_old_size, ALIGN);

    let total_new_size = match new_size.checked_add(HEADER_SIZE) {
        Some(s) => s,
        None => return ptr::null_mut(),
    };

    let new_ptr = alloc::alloc::realloc(real_ptr, old_layout, total_new_size);
    if new_ptr.is_null() {
        return ptr::null_mut();
    }

    let new_header = new_ptr as *mut AllocHeader;
    (*new_header).size = new_size;
    (*new_header).magic = HEADER_MAGIC;

    new_ptr.add(HEADER_SIZE) as *mut c_void
}

// ----------------------------------------------------------------------------
// 2. Strings & Memory (Restored for Kernel usage)
// ----------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn memcpy(dest: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let d = dest as *mut u8;
    let s = src as *const u8;
    let mut i = 0;
    while i < n {
        *d.add(i) = *s.add(i);
        i += 1;
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memmove(dest: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let d = dest as *mut u8;
    let s = src as *const u8;
    if s < d as *const u8 {
        let mut i = n;
        while i > 0 {
            i -= 1;
            *d.add(i) = *s.add(i);
        }
    } else {
        let mut i = 0;
        while i < n {
            *d.add(i) = *s.add(i);
            i += 1;
        }
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memset(s: *mut c_void, c: c_int, n: usize) -> *mut c_void {
    let d = s as *mut u8;
    let v = c as u8;
    let mut i = 0;
    while i < n {
        *d.add(i) = v;
        i += 1;
    }
    s
}

#[no_mangle]
pub unsafe extern "C" fn memcmp(s1: *const c_void, s2: *const c_void, n: usize) -> c_int {
    let s1 = core::slice::from_raw_parts(s1 as *const u8, n);
    let s2 = core::slice::from_raw_parts(s2 as *const u8, n);
    for i in 0..n {
        let diff = s1[i] as c_int - s2[i] as c_int;
        if diff != 0 {
            return diff;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn strlen(s: *const c_char) -> usize {
    let mut len = 0;
    while *s.add(len) != 0 {
        len += 1;
    }
    len
}

#[no_mangle]
pub unsafe extern "C" fn strcpy(dest: *mut c_char, src: *const c_char) -> *mut c_char {
    let mut i = 0;
    loop {
        let c = *src.add(i);
        *dest.add(i) = c;
        if c == 0 {
            break;
        }
        i += 1;
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn strncpy(dest: *mut c_char, src: *const c_char, n: usize) -> *mut c_char {
    let mut i = 0;
    while i < n {
        let c = *src.add(i);
        *dest.add(i) = c;
        if c == 0 {
            while i < n {
                *dest.add(i) = 0;
                i += 1;
            }
            break;
        }
        i += 1;
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn strcmp(s1: *const c_char, s2: *const c_char) -> c_int {
    let mut i = 0;
    loop {
        let c1 = *s1.add(i) as u8;
        let c2 = *s2.add(i) as u8;
        if c1 != c2 {
            return (c1 as c_int) - (c2 as c_int);
        }
        if c1 == 0 {
            return 0;
        }
        i += 1;
    }
}

// Removed _impure_ptr logic to move to C shim

#[repr(C)]
pub struct tms {
    pub tms_utime: c_long,
    pub tms_stime: c_long,
    pub tms_cutime: c_long,
    pub tms_cstime: c_long,
}

#[no_mangle]
pub unsafe extern "C" fn _open(name: *const c_char, flags: c_int, mode: c_int) -> c_int {
    let len = strlen(name);
    let ret = raw_syscall(ViSyscall::Open, name as usize, len, flags as usize, mode as usize);
    ret as c_int
}

#[no_mangle]
pub unsafe extern "C" fn _fcntl(fd: c_int, cmd: c_int, arg: c_int) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn _execve(name: *const c_char, argv: *const *const c_char, env: *const *const c_char) -> c_int {
    -1
}

#[no_mangle]
pub unsafe extern "C" fn _fork() -> c_int {
    -1
}

#[no_mangle]
pub unsafe extern "C" fn _wait(status: *mut c_int) -> c_int {
    -1
}

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
pub unsafe extern "C" fn _link(old: *const c_char, new: *const c_char) -> c_int {
    -1
}

#[no_mangle]
pub unsafe extern "C" fn _unlink(name: *const c_char) -> c_int {
    -1
}

#[no_mangle]
pub unsafe extern "C" fn _putchar(c: c_char) {
    let buf = [c as u8];
    // FD 1 = stdout
    raw_syscall(ViSyscall::Write, 1, buf.as_ptr() as usize, 1, 0);
}

// ----------------------------------------------------------------------------
// 3. System Call Helper
// ----------------------------------------------------------------------------
#[cfg(target_arch = "riscv64")]
#[inline(always)]
unsafe fn raw_syscall(id: ViSyscall, a0: usize, a1: usize, a2: usize, a3: usize) -> isize {
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

#[cfg(not(target_arch = "riscv64"))]
unsafe fn raw_syscall(_id: ViSyscall, _a0: usize, _a1: usize, _a2: usize, _a3: usize) -> isize {
    0
}

// ----------------------------------------------------------------------------
// 4. Newlib Syscalls (Low Level)
// ----------------------------------------------------------------------------

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

#[no_mangle]
pub unsafe extern "C" fn _write(handle: c_int, buf: *const c_void, count: usize) -> c_int {
    let ret = raw_syscall(ViSyscall::Write, handle as usize, buf as usize, count, 0);
    ret as c_int
}

#[no_mangle]
pub unsafe extern "C" fn _read(handle: c_int, buf: *mut c_void, count: usize) -> c_int {
    let ret = raw_syscall(ViSyscall::Read, handle as usize, buf as usize, count, 0);
    ret as c_int
}

#[no_mangle]
pub unsafe extern "C" fn _close(handle: c_int) -> c_int {
    // Socket fds (10–17) go through the net service; others use the kernel Close syscall.
    if handle >= SOCK_BASE_FD && handle < SOCK_BASE_FD + MAX_SOCKETS as c_int {
        return socket_close(handle);
    }
    let ret = raw_syscall(ViSyscall::Close, handle as usize, 0, 0, 0);
    ret as c_int
}

#[no_mangle]
pub unsafe extern "C" fn _lseek(handle: c_int, offset: c_long, whence: c_int) -> c_long {
    // Cast offset via isize to preserve the sign bit on all platforms including wasm32.
    // A bare `offset as usize` on a negative c_long would produce a very large positive
    // number that the kernel would misinterpret.
    let ret = raw_syscall(
        ViSyscall::Seek,
        handle as usize,
        offset as isize as usize,
        whence as usize,
        0,
    );
    ret as c_long
}

#[no_mangle]
pub unsafe extern "C" fn _fstat(handle: c_int, st: *mut stat) -> c_int {
    if !st.is_null() {
        // Zero-init all fields so callers never read uninitialised memory.
        core::ptr::write_bytes(st as *mut u8, 0, core::mem::size_of::<stat>());
        if handle <= 2 {
            (*st).st_mode = 0o20000 | 0o666; // S_IFCHR — character device (tty)
        } else {
            (*st).st_mode = 0o100000 | 0o666; // S_IFREG — regular file
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn _isatty(handle: c_int) -> c_int {
    if handle >= 0 && handle <= 2 { 1 } else { 0 }
}

#[no_mangle]
pub unsafe extern "C" fn _getpid() -> c_int {
    1
}

#[no_mangle]
pub unsafe extern "C" fn _kill(pid: c_int, sig: c_int) -> c_int {
    -1
}

#[no_mangle]
pub unsafe extern "C" fn _exit(status: c_int) -> ! {
    raw_syscall(ViSyscall::Exit, status as usize, 0, 0, 0);
    loop {}
}

#[no_mangle]
pub unsafe extern "C" fn _time(tloc: *mut c_long) -> c_long {
    let mut now: usize = 0;
    let ret = raw_syscall(ViSyscall::GetTime, 0, 0, 0, 0);
    if ret >= 0 {
        now = ret as usize;
    }
    if !tloc.is_null() {
        *tloc = now as c_long;
    }
    now as c_long
}

#[no_mangle]
pub unsafe extern "C" fn _gettimeofday(tv: *mut timeval, tz: *mut c_void) -> c_int {
    if !tv.is_null() {
        // ViSyscall::GetTime returns timestamp (likely ms since boot or something)
        // Assume milliseconds for now? Or seconds?
        // Let's assume GetTime returns seconds for this shim or simple tick.
        let ret = raw_syscall(ViSyscall::GetTime, 0, 0, 0, 0);
        if ret >= 0 {
            (*tv).tv_sec = ret as c_long;
            (*tv).tv_usec = 0;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn _sbrk(incr: c_int) -> *mut c_void {
   ptr::null_mut()
}

// ----------------------------------------------------------------------------
// 5. Entropy — getentropy(2)
// ----------------------------------------------------------------------------

/// Fill `buf` with `buflen` cryptographically random bytes via VirtIO-RNG.
///
/// Maps to `sys_get_random(214)`.  Returns 0 on success, −1 if the device is
/// absent or `buflen` exceeds the 256-byte POSIX limit.
///
/// Required by embedded-tls and any C library doing crypto (e.g. picolibc).
#[no_mangle]
pub unsafe extern "C" fn getentropy(buf: *mut c_void, buflen: usize) -> c_int {
    // POSIX specifies EINVAL when buflen > 256.
    if buf.is_null() || buflen > 256 { return -1; }
    let mut written = 0usize;
    let dst = buf as *mut u8;
    // GetRandom delivers up to 64 bytes per call — loop until satisfied.
    while written < buflen {
        let chunk = (buflen - written).min(64);
        let ret = raw_syscall(
            ViSyscall::GetRandom,
            dst.add(written) as usize,
            chunk, 0, 0,
        );
        if ret <= 0 { return -1; }
        written += ret as usize;
    }
    0
}

// ----------------------------------------------------------------------------
// 6. BSD Socket Shims — socket / connect / send / recv
// ----------------------------------------------------------------------------
//
// These shims let C libraries (picolibc, embedded-tls adapters) call familiar
// BSD socket functions.  Internally they translate to typed `NetRequest` IPC
// sent to the net service.
//
// Socket fd range: 10–17 (above stdio 0–2 and shell-reserved 3–9).
// The fd→cap_id mapping is stored in SOCK_CAPS[] (one slot per fd).
//
// Thread safety: ViCell is single-hart for G1; AtomicU32 CAS prevents
// double-alloc from interrupt context (e.g. if ever called from an ISR).

const SOCK_BASE_FD: c_int = 10;
const MAX_SOCKETS: usize = 8;

/// AF_INET
const AF_INET: c_int = 2;
/// SOCK_STREAM (TCP)
const SOCK_STREAM: c_int = 1;

/// Lazily cached TID of the net service cell.
static NET_TID_CACHE: AtomicUsize = AtomicUsize::new(0);

/// cap_id slot for each socket fd (fd 10 → index 0, …, fd 17 → index 7).
/// 0 = free, u32::MAX = reserved (alloc in progress), other = live cap_id.
static SOCK_CAPS: [AtomicU32; MAX_SOCKETS] = [
    AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0),
    AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0),
];

/// Internet socket address (mirrors `struct sockaddr_in`).
#[repr(C)]
pub struct sockaddr_in {
    pub sin_family: u16,
    /// Port in network byte order (big-endian).
    pub sin_port: u16,
    /// IPv4 address in network byte order (big-endian).
    pub sin_addr: u32,
    pub sin_zero: [u8; 8],
}

/// Resolve the net service TID, caching the result.  Returns 0 if unavailable.
fn net_tid() -> usize {
    let cached = NET_TID_CACHE.load(Ordering::Relaxed);
    if cached != 0 { return cached; }
    // ViSyscall::LookupService = 206, service::NET = 2
    let tid = unsafe { raw_syscall(ViSyscall::LookupService, 2, 0, 0, 0) };
    if tid > 0 {
        NET_TID_CACHE.store(tid as usize, Ordering::Relaxed);
        tid as usize
    } else {
        0
    }
}

/// Allocate a free socket slot and return (fd, table_index).  Returns `None` if full.
fn alloc_fd() -> Option<(c_int, usize)> {
    for i in 0..MAX_SOCKETS {
        if SOCK_CAPS[i].compare_exchange(0, u32::MAX, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            return Some((SOCK_BASE_FD + i as c_int, i));
        }
    }
    None
}

/// Look up the cap_id for `fd`, or return None if fd is invalid/free.
fn cap_from_fd(fd: c_int) -> Option<u32> {
    let idx = fd - SOCK_BASE_FD;
    if idx < 0 || idx as usize >= MAX_SOCKETS { return None; }
    let cap = SOCK_CAPS[idx as usize].load(Ordering::Acquire);
    if cap == 0 || cap == u32::MAX { None } else { Some(cap) }
}

/// Create a TCP/IP socket.  Only AF_INET + SOCK_STREAM (TCP) is supported.
/// Returns an fd in [10, 17] on success, −1 on error.
#[no_mangle]
pub unsafe extern "C" fn socket(domain: c_int, type_: c_int, protocol: c_int) -> c_int {
    if domain != AF_INET || type_ != SOCK_STREAM { return -1; }
    match alloc_fd() {
        Some((fd, _)) => fd,
        None => -1,
    }
}

/// Connect `fd` to the address in `addr` (must be `sockaddr_in`).
/// Returns 0 on success, −1 on error.
///
/// If connect fails the slot remains at u32::MAX (allocated-but-not-connected);
/// call `close(fd)` to release it.  Subsequent send/recv on the same fd will
/// return −1 until a successful connect stores a real cap_id.
#[no_mangle]
pub unsafe extern "C" fn connect(fd: c_int, addr: *const c_void, addrlen: c_int) -> c_int {
    let idx = fd - SOCK_BASE_FD;
    if idx < 0 || idx as usize >= MAX_SOCKETS { return -1; }
    if addr.is_null() || addrlen < core::mem::size_of::<sockaddr_in>() as c_int { return -1; }
    let net = net_tid();
    if net == 0 { return -1; }

    let sin = addr as *const sockaddr_in;
    if (*sin).sin_family != AF_INET as u16 { return -1; }
    // sin_addr is network byte order (big-endian); to_be_bytes() yields [a, b, c, d].
    let ip = (*sin).sin_addr.to_be_bytes();
    let port = u16::from_be((*sin).sin_port);

    let mut req_buf = [0u8; IPC_BUF_SIZE];
    let req = NetRequest::TcpConnect { addr: ip, port };
    let Ok(encoded) = encode(&req, &mut req_buf) else { return -1; };
    // Note: Recv(mask=net) would filter to responses from net only; mask=0 accepts any
    // sender.  For G1 (single active call per socket) this is safe; a future revision
    // should use the sender-filter mask once the kernel ABI is confirmed.
    raw_syscall(ViSyscall::Send, net, encoded.as_ptr() as usize, encoded.len(), 0);

    let mut resp_buf = [0u8; IPC_BUF_SIZE];
    let n = raw_syscall(ViSyscall::Recv, 0, resp_buf.as_mut_ptr() as usize, resp_buf.len(), 0);
    if n <= 0 { return -1; }

    match decode::<NetResponse>(&resp_buf[..n as usize]) {
        Ok(NetResponse::CapId(cap)) if cap > 0 => {
            SOCK_CAPS[idx as usize].store(cap, Ordering::Release);
            0
        }
        _ => -1,
    }
}

/// Send `len` bytes from `buf` over socket `fd`.
///
/// At most 495 bytes are sent per call (IPC payload ceiling after postcard
/// framing overhead); callers sending more must loop.  Returns the number of
/// bytes actually queued (≤ min(len, 495)), or −1 on error.
#[no_mangle]
pub unsafe extern "C" fn send(fd: c_int, buf: *const c_void, len: usize, flags: c_int) -> c_int {
    if buf.is_null() { return -1; }
    let Some(cap) = cap_from_fd(fd) else { return -1; };
    let net = net_tid();
    if net == 0 { return -1; }

    // Clamp to the maximum payload we can fit in one IPC frame.
    let capped = len.min(495);
    let data = core::slice::from_raw_parts(buf as *const u8, capped);
    let mut req_buf = [0u8; IPC_BUF_SIZE];
    let req = NetRequest::TcpSend { cap_id: cap, data };
    let Ok(encoded) = encode(&req, &mut req_buf) else { return -1; };
    raw_syscall(ViSyscall::Send, net, encoded.as_ptr() as usize, encoded.len(), 0);

    let mut resp_buf = [0u8; 8];
    let n = raw_syscall(ViSyscall::Recv, 0, resp_buf.as_mut_ptr() as usize, resp_buf.len(), 0);
    if n <= 0 { return -1; }
    if (n as usize) < 4 { return -1; }
    // Net service replies with bytes accepted as a little-endian i32.
    // Cap at capped so callers never believe more bytes were sent than we forwarded.
    let accepted = i32::from_le_bytes([resp_buf[0], resp_buf[1], resp_buf[2], resp_buf[3]]);
    accepted.min(capped as i32)
}

/// Receive up to `len` bytes from socket `fd` into `buf`.
/// Returns bytes received, or −1 on error.
#[no_mangle]
pub unsafe extern "C" fn recv(fd: c_int, buf: *mut c_void, len: usize, flags: c_int) -> c_int {
    if buf.is_null() { return -1; }
    let Some(cap) = cap_from_fd(fd) else { return -1; };
    let net = net_tid();
    if net == 0 { return -1; }

    let mut req_buf = [0u8; IPC_BUF_SIZE];
    let req = NetRequest::TcpRecv { cap_id: cap, buf_len: len as u32 };
    let Ok(encoded) = encode(&req, &mut req_buf) else { return -1; };
    raw_syscall(ViSyscall::Send, net, encoded.as_ptr() as usize, encoded.len(), 0);

    let mut resp_buf = [0u8; IPC_BUF_SIZE];
    let n = raw_syscall(ViSyscall::Recv, 0, resp_buf.as_mut_ptr() as usize, resp_buf.len(), 0);
    if n <= 0 { return 0; }

    match decode::<NetResponse>(&resp_buf[..n as usize]) {
        Ok(NetResponse::Data(data)) => {
            let copy_len = data.len().min(len);
            core::ptr::copy_nonoverlapping(data.as_ptr(), buf as *mut u8, copy_len);
            copy_len as c_int
        }
        _ => -1,
    }
}

/// Close a socket fd (called by `_close` for the socket fd range).
///
/// Handles two cases:
///  - cap == u32::MAX: socket was allocated (socket() succeeded) but never
///    connected; just free the slot — no TcpClose IPC needed.
///  - cap is a real cap_id: send TcpClose to the net service, then free.
unsafe fn socket_close(fd: c_int) -> c_int {
    let idx = (fd - SOCK_BASE_FD) as usize;
    let cap = SOCK_CAPS[idx].load(Ordering::Acquire);
    if cap == 0 { return -1; }   // already free

    if cap != u32::MAX {
        // Live connection — tear it down with the net service.
        let net = net_tid();
        if net != 0 {
            let mut req_buf = [0u8; IPC_BUF_SIZE];
            let req = NetRequest::TcpClose { cap_id: cap };
            if let Ok(encoded) = encode(&req, &mut req_buf) {
                raw_syscall(ViSyscall::Send, net, encoded.as_ptr() as usize, encoded.len(), 0);
                let mut r = [0u8; 4];
                raw_syscall(ViSyscall::Recv, 0, r.as_mut_ptr() as usize, r.len(), 0);
            }
        }
    }
    SOCK_CAPS[idx].store(0, Ordering::Release);
    0
}
