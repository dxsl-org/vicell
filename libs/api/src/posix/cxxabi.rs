// SPDX-License-Identifier: MPL-2.0
//! Minimal C++ ABI stubs for freestanding C++ (no exceptions, no RTTI).
//! 
//! SAFETY: These stubs ONLY support -fno-exceptions -fno-rtti compiled C++ code.
//! Do NOT attempt to use exceptions, RTTI, or STL containers — they require
//! full libcxxabi/libstdc++ which is SAS-unsafe. Use Tier 3b Linux VM instead.

#![allow(unsafe_code)]

use super::sysio::raw_syscall;
use crate::syscall::ViSyscall;

/// Called when a pure virtual function is invoked (programming error).
/// Terminates the cell immediately — equivalent to a Rust panic.
#[no_mangle]
pub unsafe extern "C" fn __cxa_pure_virtual() -> ! {
    raw_syscall(ViSyscall::Log, b"FATAL: pure virtual call\n".as_ptr() as usize, 25, 0, 0);
    raw_syscall(ViSyscall::Exit, 134, 0, 0, 0); // 134 = 128 + SIGABRT(6)
    loop {}
}

/// Thread-safe static local init guard — single-threaded stub.
/// C++ emits __cxa_guard_acquire/release around function-local statics.
/// In ViCell's single-threaded cells, a simple flag suffices.
#[no_mangle]
pub unsafe extern "C" fn __cxa_guard_acquire(guard: *mut u64) -> i32 {
    if *guard == 0 { 1 } else { 0 }  // 1 = needs init, 0 = already done
}

#[no_mangle]
pub unsafe extern "C" fn __cxa_guard_release(guard: *mut u64) {
    *guard = 1;  // mark initialized
}

#[no_mangle]
pub unsafe extern "C" fn __cxa_guard_abort(_guard: *mut u64) {
    // Init failed — leave guard at 0 so next attempt retries
}

/// abort() — terminates cell immediately. No cleanup, no atexit handlers.
/// This is the correct behavior for SAS: kernel reclaims all resources.
#[no_mangle]
pub unsafe extern "C" fn abort() -> ! {
    raw_syscall(ViSyscall::Exit, 134, 0, 0, 0);
    loop {}
}
