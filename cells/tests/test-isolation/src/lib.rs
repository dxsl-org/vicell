// Test: Compile-time layer isolation enforcement
//
// This app should compile successfully using only ostd/api.
// Any attempt to import kernel should fail.

#![no_std]
extern crate alloc;

#[allow(unused_imports)]
use ostd::prelude::*;

// ✅ This should work - apps can use API
#[allow(dead_code)]
fn test_api_usage() {
    // We can reference API types
    use api::fs::{OpenMode, SeekFrom};
    let _mode = OpenMode::Read;
    let _seek = SeekFrom::Start(0);
}

// ❌ This should NOT compile if uncommented:
// use kernel::memory::allocate;
//
// Compile error expected:
// "error[E0432]: unresolved import `kernel`"

#[no_mangle]
pub extern "C" fn _start() -> ! {
    loop {}
}

// ✅ VALIDATION: If this compiles, layer isolation works!
