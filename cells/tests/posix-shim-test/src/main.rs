//! POSIX shim integration test cell.
//!
//! Tests the C ABI shims in `libs/api/src/posix.rs`:
//!   - getentropy(2) via sys_get_random (opcode 214)
//!   - socket / connect / send / recv / close via typed Net IPC
//!
//! Spawn with: `posix-shim-test` from the shell.
//! The integration test in tests/integration/tests/boot.rs checks for:
//!   "POSIX-ENTROPY: OK" and "POSIX-NET: OK"

#![no_std]
#![no_main]
extern crate alloc;
extern crate ostd;

use alloc::format;
use core::ffi::c_void;
use ostd::io::println;

// Network shim test connects to the QEMU SLIRP host echo server.
// Port must match POSIX_SHIM_ECHO_PORT in boot.rs.
const ECHO_IP: [u8; 4] = [10, 0, 2, 2];
const ECHO_PORT: u16 = 10009;

api::declare_manifest!(block_io = false, network = false, spawn = false);
// GetRandom needed for getentropy; Send/Recv/LookupService for net-service IPC.
api::declare_syscalls![Send, Recv, Log, LookupService, GetRandom];

// Declare C ABI directly — works whether the symbols come from api::posix (Tier A)
// or mlibc/libc.a (Tier B); avoids Rust feature-unification breaking the lookup.
#[repr(C)]
struct SockaddrIn {
    sin_family: u16,
    sin_port:   u16,
    sin_addr:   u32,
    sin_zero:   [u8; 8],
}

extern "C" {
    fn getentropy(buf: *mut c_void, buflen: usize) -> i32;
    fn socket(domain: i32, typ: i32, protocol: i32) -> i32;
    fn connect(fd: i32, addr: *const c_void, addrlen: i32) -> i32;
    fn send(fd: i32, buf: *const c_void, len: usize, flags: i32) -> isize;
    fn recv(fd: i32, buf: *mut c_void, len: usize, flags: i32) -> isize;
    #[link_name = "_close"]
    fn close(fd: i32);
}

#[no_mangle]
pub fn main() {
    test_getentropy();
    test_net();
}

fn test_getentropy() {
    let mut buf = [0u8; 16];
    // SAFETY: buf is a valid 16-byte stack buffer; shim validates len ≤ 256.
    let ret = unsafe { getentropy(buf.as_mut_ptr() as *mut c_void, 16) };
    if ret == 0 && buf.iter().any(|b| *b != 0) {
        println("[posix-shim] POSIX-ENTROPY: OK");
    } else {
        println(&format!("[posix-shim] POSIX-ENTROPY: FAIL ret={ret}"));
    }
}

fn test_net() {
    // AF_INET=2, SOCK_STREAM=1, protocol=0
    let fd = unsafe { socket(2, 1, 0) };
    if fd < 0 {
        println("[posix-shim] POSIX-NET: FAIL socket");
        return;
    }

    let addr = SockaddrIn {
        sin_family: 2u16,
        sin_port:   ECHO_PORT.to_be(),
        sin_addr:   u32::from_be_bytes(ECHO_IP),
        sin_zero:   [0u8; 8],
    };
    // SAFETY: addr is a valid SockaddrIn on the stack; addrlen matches.
    let ret = unsafe {
        connect(fd, &addr as *const _ as *const c_void, core::mem::size_of::<SockaddrIn>() as i32)
    };
    if ret < 0 {
        println("[posix-shim] POSIX-NET: FAIL connect");
        unsafe { close(fd); }
        return;
    }

    let msg = b"hello\n";
    let sent = unsafe { send(fd, msg.as_ptr() as *const c_void, msg.len(), 0) };
    if sent < 0 {
        println(&format!("[posix-shim] POSIX-NET: FAIL send sent={sent}"));
        unsafe { close(fd); }
        return;
    }

    let mut rbuf = [0u8; 64];
    let mut n: isize = -1;
    for _ in 0..2000 {
        n = unsafe { recv(fd, rbuf.as_mut_ptr() as *mut c_void, rbuf.len(), 0) };
        if n > 0 { break; }
        ostd::syscall::sys_yield();
    }
    unsafe { close(fd); }

    if n > 0 {
        println("[posix-shim] POSIX-NET: OK");
    } else {
        println(&format!("[posix-shim] POSIX-NET: FAIL recv n={n}"));
    }
}
