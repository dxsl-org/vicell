//! C runtime smoke test: validates math/stdio/setjmp posix shim linkage.
//!
//! Calls the `#[no_mangle] extern "C"` symbols from libs/api/src/posix/ via
//! Rust extern declarations — if any symbol is missing the linker will fail.
//! Expected output (integration test marker):
//!   "C-MATH-SMOKE: 7/7 pass"
//!   "C-MATH-SMOKE: setjmp OK"

#![no_std]
#![no_main]
extern crate api;
extern crate ostd;
use ostd::syscall::sys_exit;

use core::ffi::c_void;

api::declare_manifest!(block_io = false, network = false, spawn = false);
api::declare_syscalls![Write, Log, Exit];

// ---------------------------------------------------------------------------
// Math symbols — all backed by Rust libm in posix/math.rs
// ---------------------------------------------------------------------------

extern "C" {
    fn sin(x: f64) -> f64;
    fn cos(x: f64) -> f64;
    fn sqrt(x: f64) -> f64;
    fn pow(x: f64, y: f64) -> f64;
    fn log(x: f64) -> f64;
    fn atan2(y: f64, x: f64) -> f64;
    fn sinf(x: f32) -> f32;
    fn printf(fmt: *const u8, ...) -> i32;
}

// ---------------------------------------------------------------------------
// setjmp/longjmp — arch asm in posix/setjmp.rs
// ---------------------------------------------------------------------------

const JMP_BUF_WORDS: usize = 22; // enough for both riscv64 (16) and aarch64 (22)

#[repr(C)]
struct JmpBuf([u64; JMP_BUF_WORDS]);

extern "C" {
    fn setjmp(env: *mut JmpBuf) -> i32;
    fn longjmp(env: *const JmpBuf, val: i32) -> !;
}

// ---------------------------------------------------------------------------
// POSIX _write for direct output fallback
// ---------------------------------------------------------------------------

fn write_bytes(s: &[u8]) {
    extern "C" { fn _write(fd: i32, buf: *const c_void, len: usize) -> i32; }
    unsafe { _write(1, s.as_ptr() as *const c_void, s.len()); }
}

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

const EPS: f64 = 1e-9;

fn near(a: f64, b: f64) -> bool { (a - b).abs() < EPS }
fn nearf(a: f32, b: f32) -> bool { (a - b).abs() < 1e-6 }

const M_E: f64 = core::f64::consts::E;
const M_PI: f64 = core::f64::consts::PI;

#[no_mangle]
pub fn main() {
    let mut pass = 0i32;
    let total = 7i32;

    // Test 1: sin(0) == 0
    if near(unsafe { sin(0.0) }, 0.0)      { pass += 1; } else { write_bytes(b"FAIL sin(0)\n"); }
    // Test 2: cos(0) == 1
    if near(unsafe { cos(0.0) }, 1.0)      { pass += 1; } else { write_bytes(b"FAIL cos(0)\n"); }
    // Test 3: sqrt(4) == 2
    if near(unsafe { sqrt(4.0) }, 2.0)     { pass += 1; } else { write_bytes(b"FAIL sqrt(4)\n"); }
    // Test 4: pow(2, 10) == 1024
    if near(unsafe { pow(2.0, 10.0) }, 1024.0) { pass += 1; } else { write_bytes(b"FAIL pow(2,10)\n"); }
    // Test 5: sinf(0) == 0
    if nearf(unsafe { sinf(0.0f32) }, 0.0f32) { pass += 1; } else { write_bytes(b"FAIL sinf(0)\n"); }
    // Test 6: log(e) == 1
    if near(unsafe { log(M_E) }, 1.0)      { pass += 1; } else { write_bytes(b"FAIL log(e)\n"); }
    // Test 7: atan2(1,1) == pi/4
    if near(unsafe { atan2(1.0, 1.0) }, M_PI / 4.0) { pass += 1; } else { write_bytes(b"FAIL atan2(1,1)\n"); }

    // Print summary via printf (tests the stdio shim)
    unsafe {
        printf(b"C-MATH-SMOKE: %d/%d pass\n\0".as_ptr(), pass, total);
    }

    // Test setjmp/longjmp
    let mut buf = JmpBuf([0u64; JMP_BUF_WORDS]);
    let landed = unsafe { setjmp(&mut buf as *mut JmpBuf) };
    if landed == 0 {
        // First call — jump back to this point with val=42
        unsafe { longjmp(&buf as *const JmpBuf, 42) };
    }
    // landed == 42 here (longjmp returned us here)
    if landed == 42 {
        write_bytes(b"C-MATH-SMOKE: setjmp OK\n");
    } else {
        write_bytes(b"C-MATH-SMOKE: setjmp FAIL\n");
    }

    sys_exit(if pass == total { 0 } else { 1 } as usize);
}
