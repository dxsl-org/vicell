#![no_std]
#![no_main]

extern crate ostd;
extern crate api;

mod net_bridge;

/// Net service cell endpoint (boot order: vfs=3, config=4, input=5, net=6).
const VFS_ENDPOINT: usize = 3;
const OP_READ: u8 = 8;

// MicroPython embed API — provided by embed_util.c (ports/embed/port).
extern "C" {
    fn mp_embed_init(gc_heap: *mut u8, gc_heap_size: usize, stack_top: *mut u8);
    fn mp_embed_deinit();

    /// Blocking REPL loop. Returns when the interpreter exits (Ctrl-D / sys.exit).
    fn pyexec_friendly_repl() -> i32;

    /// Execute a NUL-terminated Python source string (used for -c and script mode).
    fn mp_embed_exec_str(src: *const u8);
}

/// GC heap for the Python interpreter.
static mut HEAP: [u8; 256 * 1024] = [0u8; 256 * 1024];

/// Read up to `buf.len()` bytes from a VFS path via OP_READ IPC.
///
/// Returns byte count (zero-scan from reply; sys_recv returns sender_id, not length).
/// Matches the pattern used by the Lua cell's `vfs_read_to_buf`.
fn vfs_read_to_buf(path: &str, buf: &mut [u8]) -> usize {
    let pb = path.as_bytes();
    let pl = pb.len().min(253) as u8;
    let mut req = [0u8; 256];
    req[0] = OP_READ;
    req[1] = pl;
    req[2..2 + pl as usize].copy_from_slice(&pb[..pl as usize]);
    ostd::syscall::sys_send(VFS_ENDPOINT, &req[..2 + pl as usize]);
    buf.fill(0);
    match ostd::syscall::sys_recv(0, buf) {
        ostd::syscall::SyscallResult::Ok(_) =>
            buf.iter().rposition(|&b| b != 0).map(|i| i + 1).unwrap_or(0),
        _ => 0,
    }
}

#[no_mangle]
pub extern "C" fn main(_argc: isize, _argv: *const *const u8) -> isize {
    // Always initialise the interpreter before executing any code.
    // Stack-anchor gives the GC root scanner a top-of-stack marker.
    let stack_anchor: u8 = 0;

    // SAFETY: HEAP is a `static mut` array used exclusively here. This cell is
    // single-threaded with no concurrent access; mp_embed_init takes sole
    // ownership of the buffer for the lifetime of the interpreter.
    #[allow(static_mut_refs)]
    unsafe {
        mp_embed_init(
            HEAP.as_mut_ptr(),
            HEAP.len(),
            &stack_anchor as *const u8 as *mut u8,
        );
    }

    // Read the command line published by the spawner (e.g. the shell).
    let mut argbuf = [0u8; 512];
    let n = ostd::syscall::sys_spawn_args(&mut argbuf);
    let args = core::str::from_utf8(&argbuf[..n]).unwrap_or("").trim();

    // `python -c <code>` — evaluate and park.
    if let Some(code) = args.strip_prefix("-c ").or_else(|| args.strip_prefix("-c")) {
        let code = code.trim_start();
        let clen = code.len().min(511);
        // NUL-terminate for mp_embed_exec_str.
        let mut cbuf = [0u8; 512]; // last byte stays 0 = NUL terminator
        cbuf[..clen].copy_from_slice(&code.as_bytes()[..clen]);
        // SAFETY: cbuf is NUL-terminated; mp_embed_exec_str compiles + runs the source.
        unsafe { mp_embed_exec_str(cbuf.as_ptr()) };
        // Park: cell-exit does not yet unmap the SAS segment; keep the cell alive.
        loop { ostd::task::yield_now(); }
    }

    // `python /path/to/script.py` — read from VFS and execute.
    if !args.is_empty() {
        let path = args;
        // +1 for NUL terminator required by mp_embed_exec_str.
        let mut file_buf = [0u8; 4097];
        let n = vfs_read_to_buf(path, &mut file_buf[..4096]);
        if n == 0 {
            ostd::io::print("python: cannot open '");
            ostd::io::print(path);
            ostd::io::println("'");
        } else {
            file_buf[n] = 0; // NUL-terminate
            // SAFETY: file_buf[0..n+1] is valid Python source; NUL at n.
            unsafe { mp_embed_exec_str(file_buf.as_ptr()) };
        }
        loop { ostd::task::yield_now(); }
    }

    // No args: interactive REPL.
    ostd::io::println("MicroPython v1.24.1 on ViOS (Cellular SAS)");
    ostd::io::println("Type \"help()\" for more information.");
    // SAFETY: interpreter is initialised above; pyexec_friendly_repl runs the REPL.
    unsafe { pyexec_friendly_repl() };

    // SAFETY: interpreter is initialised; deinit tears down the GC and VM.
    unsafe { mp_embed_deinit() };

    0
}
