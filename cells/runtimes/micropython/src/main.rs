#![no_std]
#![no_main]

extern crate ostd;
extern crate api;

use ostd::prelude::*;

// MicroPython embed API — provided by embed_util.c (ports/embed/port).
// mp_embed_init  : initialise GC heap + Python runtime
// mp_embed_deinit: teardown
// pyexec_friendly_repl: interactive REPL (shared/runtime/pyexec.c)
extern "C" {
    fn mp_embed_init(
        gc_heap: *mut u8,
        gc_heap_size: usize,
        stack_top: *mut u8,
    );
    fn mp_embed_deinit();

    /// Blocking REPL loop. Returns when the interpreter exits (Ctrl-D / sys.exit).
    fn pyexec_friendly_repl() -> i32;

    /// Execute a Python string (used for startup script if desired).
    fn mp_embed_exec_str(src: *const u8);
}

/// GC heap for the Python interpreter.
/// 256 KB is sufficient for most REPL workloads.
static mut HEAP: [u8; 256 * 1024] = [0u8; 256 * 1024];

#[no_mangle]
pub extern "C" fn main(_argc: isize, _argv: *const *const u8) -> isize {
    println("MicroPython v1.24.1 on ViOS (Cellular SAS)");
    println("Type \"help()\" for more information.");

    // Use a local variable's address as the stack-top marker for the GC
    // root scanner. The scanner walks from here up to the stack base
    // recorded by mp_stack_set_top inside mp_embed_init.
    let stack_anchor: u8 = 0;

    // SAFETY: HEAP is a static array only touched here; no concurrent access
    // possible because this cell runs single-threaded.
    unsafe {
        mp_embed_init(
            HEAP.as_mut_ptr(),
            HEAP.len(),
            &stack_anchor as *const u8 as *mut u8,
        );

        pyexec_friendly_repl();

        mp_embed_deinit();
    }

    0
}
