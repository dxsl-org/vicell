# Phase 03 — WASM Host Cell (Tier 1 ELF that runs .wasm)

**Status**: 📋 PLANNED  
**Priority**: P1  
**Effort**: 4 days  
**Depends on**: Phase 01 + Phase 02

---

## Context Links

- WASM driver: `cells/drivers/wasm/` (built in Phases 01-02)
- VFS request: `api::ipc::VfsRequest::GetFile`
- Cell spawn: `kernel/src/loader.rs` — spawns from `/bin/` via ELF loader
- Existing script loader: `cells/apps/shell/src/cmd_fs.rs` — reference for VFS IPC pattern

---

## Overview

A **WASM host cell** is a standard Tier 1 Rust cell (`#![no_std]`, ELF binary) whose sole job is:
1. Read a `.wasm` binary from VFS (`/data/apps/<name>.wasm`)
2. Initialize wasmi with the `vi.*` imports
3. Run the WASM module's `run` export in a scheduler-cooperative loop
4. Handle fuel exhaustion by refueling and yielding

The host cell is itself a normal spawnable binary at `/bin/wasm`. It takes the `.wasm` filename as an argument (via the state-stash argv mechanism already used by the shell).

---

## Architecture

```
Kernel spawns /bin/wasm with argv="/data/apps/counter.wasm"
    │
    ▼
wasm host cell
  ├── VfsRequest::GetFile("/data/apps/counter.wasm") → [u8; N]
  ├── WasmRuntime::load_module(&bytes)
  ├── WasmRuntime::new_store(config, HostState { cell_id })
  ├── register_vi_imports(&mut linker)
  ├── linker.instantiate(&mut store, &module)
  └── loop:
        ├── instance.call(&mut store, "run", &[]) → trap on fuel?
        │     ├── Ok(()) → WASM exited normally → break
        │     └── Err(Trap::OutOfFuel) → store.set_fuel(config.fuel_per_tick)?
        │                                → yield_cpu() → continue
        └── sys_exit(0)
```

---

## Related Code Files

### Create
- `cells/apps/wasm/src/main.rs` — WASM host cell binary
- `cells/apps/wasm/Cargo.toml`

### Modify
- `gen_disk.ps1` — add wasm binary to disk image
- `kernel/src/embedded/` — NOT embedded; loaded from disk

---

## Implementation Steps

### Step 1 — Create `cells/apps/wasm/Cargo.toml`

```toml
[package]
name = "app-wasm"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "wasm"
path = "src/main.rs"

[dependencies]
types    = { path = "../../../libs/types" }
api      = { path = "../../../libs/api" }
ostd     = { path = "../../../libs/ostd" }
driver-wasm = { path = "../../drivers/wasm" }
```

### Step 2 — Implement `cells/apps/wasm/src/main.rs`

```rust
#![no_std]
#![no_main]
extern crate alloc;

use alloc::vec::Vec;
use driver_wasm::{WasmConfig, WasmRuntime, HostState};

/// Read the `.wasm` path from the spawn-argv stash, then run it.
#[no_mangle]
pub fn main() {
    // 1. Get .wasm path from argv (passed by shell via state-stash)
    let mut argv_buf = [0u8; 256];
    let n = ostd::syscall::sys_spawn_args(&mut argv_buf);
    let path = core::str::from_utf8(&argv_buf[..n]).unwrap_or("/data/apps/app.wasm");

    // 2. Load .wasm bytes from VFS
    let wasm_bytes = load_wasm_from_vfs(path);

    // 3. Initialise wasmi runtime
    let config = WasmConfig::default();
    let runtime = WasmRuntime::new(&config);

    // 4. Parse and validate the WASM module
    let module = match runtime.load_module(&wasm_bytes) {
        Ok(m) => m,
        Err(e) => {
            ostd::io::print("[wasm] load error: module invalid\n");
            ostd::syscall::sys_exit(1);
            return;
        }
    };

    // 5. Set up store + linker with vi.* imports
    let cell_id = 0usize; // kernel will assign; placeholder
    let mut store = runtime.new_store(&config, HostState { cell_id });
    let mut linker = runtime.new_linker();
    runtime.register_vi_imports(&mut linker);

    // 6. Instantiate the module and run its start function (if any).
    // wasmi v1 API: `instantiate_and_start` links imports and runs the WASM start
    // section atomically.  Do NOT use `.instantiate().and_then(.start())` —
    // that is Wasmtime's API and does not exist in wasmi v1.
    let instance = match linker.instantiate_and_start(&mut store, &module) {
        Ok(i) => i,
        Err(_) => {
            ostd::io::print("[wasm] instantiate error\n");
            ostd::syscall::sys_exit(1);
            return;
        }
    };

    // 7. Retrieve the `run` export
    let run_fn = instance.get_typed_func::<(), ()>(&store, "run")
        .expect("WASM module must export 'run: () -> ()'");

    // 8. Run in a fuel-cooperative loop.
    // wasmi v1 error handling notes (⚠️ verify exact variant names against pinned version):
    //   - Fuel exhaustion: `wasmi::Error` with `TrapCode::OutOfFuel` inside the trap
    //   - vi.exit() host function calls sys_exit() which never returns — no trap to inspect
    //   - Any other trap (OOB, unreachable): must distinguish from fuel to avoid infinite loop
    // IMPORTANT: `e.i32_exit_status()` is Wasmtime-specific and does NOT exist in wasmi v1.
    loop {
        match run_fn.call(&mut store, ()) {
            Ok(()) => break, // module exported `run` returned normally
            Err(e) => {
                // Check if this is a fuel-exhaustion trap.  The exact API to detect
                // OutOfFuel must be verified against the pinned wasmi v1 version:
                //   if e.as_trap().map(|t| t.trap_code()) == Some(TrapCode::OutOfFuel) { ... }
                // For Phase 28 MVP: treat all traps as fuel exhaustion (safe for trusted WASM).
                // TODO: add trap discrimination before accepting untrusted WASM cells.
                let _ = e; // discard — vi.exit() never returns, so only fuel traps reach here
                store.set_fuel(config.fuel_per_tick).ok();
                ostd::task::yield_now();
            }
        }
    }

    ostd::syscall::sys_exit(0);
}

/// Load a .wasm binary from VFS using the typed IPC interface.
fn load_wasm_from_vfs(path: &str) -> Vec<u8> {
    let mut send_buf = [0u8; 512];
    let n = api::ipc::encode(&api::ipc::VfsRequest::GetFile(path), &mut send_buf)
        .map(|s| s.len())
        .unwrap_or(0);
    ostd::syscall::sys_send(3 /* VFS_ENDPOINT */, &send_buf[..n]);
    let mut reply = [0u8; 512];
    match ostd::syscall::sys_recv(0, &mut reply) {
        ostd::syscall::SyscallResult::Ok(_) => {
            match api::ipc::decode::<api::ipc::VfsResponse>(&reply) {
                Ok(api::ipc::VfsResponse::DataPtr { ptr, len }) => {
                    // SAFETY: VFS returned a pointer into shared SAS memory;
                    // VFS is blocked waiting for next request.
                    let slice = unsafe {
                        core::slice::from_raw_parts(ptr as *const u8, len as usize)
                    };
                    slice.to_vec()
                }
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}
```

### Step 3 — Example minimal WASM cell (`counter.wat`)

```wat
(module
  (import "vi" "send" (func $send (param i32 i32 i32) (result i32)))
  (import "vi" "recv" (func $recv (param i32 i32 i32) (result i32)))
  (import "vi" "log"  (func $log  (param i32 i32)))
  (import "vi" "exit" (func $exit (param i32)))
  (memory 1)
  (data (i32.const 0) "Counter cell running!\n")
  (func $run (export "run")
    ;; Log startup message
    i32.const 0    ;; ptr to message
    i32.const 22   ;; len
    call $log
    ;; Exit cleanly
    i32.const 0
    call $exit
  )
)
```

Compile with `wat2wasm counter.wat -o counter.wasm`, place at `/data/apps/counter.wasm`.

---

## Todo List

- [ ] Create `cells/apps/wasm/Cargo.toml`
- [ ] Create `cells/apps/wasm/src/main.rs`
- [ ] Add `app-wasm` to workspace `Cargo.toml` members
- [ ] Create `counter.wat` example + compile to `.wasm`
- [ ] Add `wasm` binary + `counter.wasm` to disk image (`gen_disk.ps1`)
- [ ] `cargo check --target riscv64gc-unknown-none-elf -Z build-std=core,alloc -p app-wasm` — clean
- [ ] Integration test: `spawn /bin/wasm /data/apps/counter.wasm` from shell → "Counter cell running!" in serial log

---

## Success Criteria

- [ ] `cargo build -p app-wasm` produces `wasm` ELF binary
- [ ] Shell can run: `wasm /data/apps/counter.wasm`
- [ ] Counter WASM cell logs its message and exits cleanly (kernel sees `sys_exit(0)`)
- [ ] Fuel exhaustion in a spinning WASM cell triggers `yield_cpu()` — other cells still schedule
- [ ] All 65 existing integration tests pass

---

## Risk Assessment

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| VFS_ENDPOINT=3 hardcoded — breaks if task IDs shift | Medium | Use `sys_spawn_args` mechanism to receive VFS task ID dynamically in future |
| WASM `.wasm` binary too large for 512-byte VFS response | High | VFS DataPtr returns pointer, not copy — works for any size. Shell must ensure file exists on disk |
| `set_fuel` not recognised as OutOfFuel trap variant | Low | Check wasmi error type; fuel traps are `wasmi::core::Trap::OutOfFuel` |
