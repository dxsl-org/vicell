//! mlibc Tier B integration smoke test.
//!
//! Calls mlibc C symbols (printf, malloc/free, clock_gettime) via extern "C".
//! If any symbol is missing the linker fails at build time — the cell acts as a
//! compile+link proof that:
//!   1. posix.rs Tier A symbols are suppressed (api/mlibc feature)
//!   2. mlibc's libc.a symbols are available (mlibc-shim)
//!   3. The ViCell sysdeps (vicell/syscall.h) dispatch correctly
//!
//! Expected output:
//!   "MLIBC-SMOKE: malloc OK"
//!   "MLIBC-SMOKE: printf OK"
//!   "MLIBC-SMOKE: clock OK"
//!   "MLIBC-SMOKE: 3/3 pass"

#![no_std]
#![no_main]
extern crate api;
extern crate ostd;
extern crate mlibc_shim;

use ostd::syscall::sys_exit;

api::declare_manifest!(block_io = false, network = false, spawn = false);
api::declare_syscalls![Write, Log, Exit, GetTime];

// ─── mlibc C symbols ─────────────────────────────────────────────────────────

extern "C" {
    // stdio
    fn printf(fmt: *const u8, ...) -> i32;
    // allocator (backed by sys_anon_allocate bump arena)
    fn malloc(size: usize) -> *mut u8;
    fn free(ptr: *mut u8);
    // time
    fn clock_gettime(clk_id: i32, tp: *mut TimeSpec) -> i32;
}

#[repr(C)]
struct TimeSpec {
    tv_sec:  i64,
    tv_nsec: i64,
}

// CLOCK_REALTIME = 0, CLOCK_MONOTONIC = 1
const CLOCK_MONOTONIC: i32 = 1;

// ─── Entry point ─────────────────────────────────────────────────────────────

#[no_mangle]
pub fn main() {
    let mut pass = 0i32;
    let total = 3i32;

    // Test 1: malloc/free via mlibc's frg::slab_allocator → sys_anon_allocate
    unsafe {
        let ptr = malloc(64);
        if !ptr.is_null() {
            // Write a known pattern and verify it reads back
            *ptr = 0xAB;
            if *ptr == 0xAB {
                pass += 1;
                printf(b"MLIBC-SMOKE: malloc OK\n\0".as_ptr());
            } else {
                printf(b"MLIBC-SMOKE: malloc FAIL (bad read)\n\0".as_ptr());
            }
            free(ptr);
        } else {
            printf(b"MLIBC-SMOKE: malloc FAIL (null)\n\0".as_ptr());
        }
    }

    // Test 2: printf via mlibc's Grisu3 formatter → sys_write sysdep
    unsafe {
        let n = printf(b"MLIBC-SMOKE: printf OK (val=%d)\n\0".as_ptr(), 42i32);
        if n > 0 {
            pass += 1;
        } else {
            printf(b"MLIBC-SMOKE: printf FAIL\n\0".as_ptr());
        }
    }

    // Test 3: clock_gettime (CLOCK_MONOTONIC) → sys_clock_get → GetTime op=0
    unsafe {
        let mut ts = TimeSpec { tv_sec: 0, tv_nsec: 0 };
        let ret = clock_gettime(CLOCK_MONOTONIC, &mut ts as *mut TimeSpec);
        // Kernel always returns a positive tick count; tv_sec or tv_nsec must be > 0
        if ret == 0 && (ts.tv_sec > 0 || ts.tv_nsec > 0) {
            pass += 1;
            printf(b"MLIBC-SMOKE: clock OK (sec=%lld)\n\0".as_ptr(), ts.tv_sec);
        } else {
            printf(b"MLIBC-SMOKE: clock FAIL (ret=%d sec=%lld)\n\0".as_ptr(),
                   ret, ts.tv_sec);
        }
    }

    unsafe {
        printf(b"MLIBC-SMOKE: %d/%d pass\n\0".as_ptr(), pass, total);
    }

    sys_exit(if pass == total { 0 } else { 1 } as usize);
}
