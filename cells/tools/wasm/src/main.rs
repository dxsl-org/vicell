//! WASM host cell — runs WebAssembly (.wasm) binaries as Tier 2 cells.
//!
//! Usage: spawn `/bin/wasm` with the `.wasm` file path as argv.
//! The shell sets this via `sys_set_spawn_args("/data/apps/app.wasm")`.

#![no_std]
#![no_main]
extern crate alloc;

use alloc::vec::Vec;
use driver_wasm::{WasmConfig, WasmRuntime, HostState};
use driver_wasm::imports::register_vi_imports;

api::declare_syscalls![Send, Recv, Log, Heartbeat];

/// VFS service task ID — boot order: init=1, user_hello=2, vfs=3.
const VFS_ENDPOINT: usize = 3;

#[no_mangle]
pub fn main() {
    // 1. Read .wasm path from spawn-argv stash (set by shell before spawning).
    let mut argv_buf = [0u8; 256];
    let n = ostd::syscall::sys_spawn_args(&mut argv_buf);
    let path = if n > 0 {
        core::str::from_utf8(&argv_buf[..n]).unwrap_or("/data/apps/app.wasm")
    } else {
        "/data/apps/app.wasm"
    };

    // 2. Load .wasm binary from VFS.
    let wasm_bytes = load_from_vfs(path);
    if wasm_bytes.is_empty() {
        ostd::io::print("[wasm] error: could not read ");
        ostd::io::println(path);
        ostd::syscall::sys_exit(1);
        // sys_exit diverges; unreachable but satisfies the compiler with no warning
    }

    // 3. Initialise wasmi with fuel metering.
    let config = WasmConfig::default();
    let runtime = WasmRuntime::new(&config);

    // 4. Parse and validate the WASM module.
    let module = match runtime.load_module(&wasm_bytes) {
        Ok(m) => m,
        Err(_) => {
            ostd::io::print("[wasm] error: invalid WASM module at ");
            ostd::io::println(path);
            ostd::syscall::sys_exit(1);
        }
    };

    // 5. Create store + linker and register vi.* imports.
    let mut store = runtime.new_store(&config, HostState { cell_task_id: 0 });
    let mut linker = runtime.new_linker();
    register_vi_imports(&mut linker);

    // 6. Instantiate the module and run its start function (if any).
    //    wasmi v1: use `instantiate_and_start` — NOT `.instantiate().and_then(.start())`.
    let instance = match linker.instantiate_and_start(&mut store, &module) {
        Ok(i) => i,
        Err(_) => {
            ostd::io::print("[wasm] error: instantiation failed\n");
            ostd::syscall::sys_exit(1);
        }
    };

    // 7. Retrieve the exported `run` function.
    let run_fn = match instance.get_typed_func::<(), ()>(&store, "run") {
        Ok(f) => f,
        Err(_) => {
            ostd::io::print("[wasm] error: module must export 'run: () -> ()'\n");
            ostd::syscall::sys_exit(1);
        }
    };

    // 8. Run in a fuel-cooperative loop.
    //    vi.exit() calls sys_exit() which never returns, so we only reach
    //    this loop body on fuel exhaustion or the module's run() returning.
    //    NOTE: verify exact wasmi v1 OutOfFuel error variant before extending
    //    this to distinguish trap types for untrusted WASM cells.
    loop {
        match run_fn.call(&mut store, ()) {
            Ok(()) => break, // module returned normally from run()
            Err(_) => {
                // For Phase 28 MVP (trusted WASM only): treat all traps as fuel
                // exhaustion since vi.exit() never returns to this point.
                store.set_fuel(config.fuel_per_tick).ok();
                ostd::task::yield_now();
            }
        }
    }

    ostd::syscall::sys_exit(0);
}

/// Read a `.wasm` binary from the VFS service via typed IPC.
///
/// Uses `VfsResponse::DataPtr` for zero-copy access to VFS memory.
fn load_from_vfs(path: &str) -> Vec<u8> {
    let mut send_buf = [0u8; 512];
    let n = match api::ipc::encode(&api::ipc::VfsRequest::GetFile(path), &mut send_buf) {
        Ok(s) => s.len(),
        Err(_) => return Vec::new(),
    };
    ostd::syscall::sys_send(VFS_ENDPOINT, &send_buf[..n]);

    let mut reply = [0u8; 512];
    // sys_recv always returns SyscallResult::Ok(sender_id) — the Err arm is
    // unreachable with the current ostd API.  If VFS isn't ready or replies
    // with an unexpected response, postcard decode will fail → empty Vec.
    match ostd::syscall::sys_recv(0, &mut reply) {
        ostd::syscall::SyscallResult::Ok(sender) if sender > 0 => {
            match api::ipc::decode::<api::ipc::VfsResponse>(&reply) {
                Ok(api::ipc::VfsResponse::DataPtr { ptr, len }) => {
                    // SAFETY: VFS replied with a pointer into SAS-shared VFS memory.
                    // The decode succeeded, confirming this is a valid DataPtr response.
                    // VFS is blocked waiting for its next sys_recv, so no concurrent
                    // modification of the pointed-to memory can occur while we copy.
                    unsafe {
                        core::slice::from_raw_parts(ptr as *const u8, len as usize).to_vec()
                    }
                }
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}
