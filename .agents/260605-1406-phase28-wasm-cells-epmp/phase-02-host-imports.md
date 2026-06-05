# Phase 02 — WASM Host Import Functions (vi.* IPC Bridge)

**Status**: 📋 PLANNED  
**Priority**: P1  
**Effort**: 3 days  
**Depends on**: Phase 01

---

## Context Links

- wasmi Linker API: https://docs.rs/wasmi/latest/wasmi/struct.Linker.html
- Caller memory access: `Caller::data()` + `Caller::get_export("memory")`
- Current IPC: `kernel/src/task.rs` — `ipc_send()`, `ipc_recv()`, `yield_cpu()`
- ostd syscall wrappers: `libs/ostd/src/syscall.rs`

---

## Overview

WASM cells cannot call ViCell syscalls directly — they run inside the wasmi interpreter which intercepts all external calls. This phase implements 4 host import functions that bridge the WASM cell's `vi.*` imports to ViCell's IPC syscall layer.

The 4 imports live in the `vi` module namespace:

| WASM import | Signature | Behavior |
|-------------|-----------|----------|
| `vi.send` | `(i32 target, i32 ptr, i32 len) → i32` | Encode bytes from WASM linear memory, call `sys_send` |
| `vi.recv` | `(i32 ptr, i32 max_len, i32 sender_out) → i32` | `sys_recv` into WASM linear memory, write sender_id |
| `vi.log` | `(i32 ptr, i32 len)` | UTF-8 log to kernel serial via `println` |
| `vi.exit` | `(i32 code)` | Call `sys_exit(code)` — WASM cell shuts down |

---

## Architecture

```
WASM cell .wasm           wasmi interpreter         ViCell kernel
────────────────          ─────────────────────     ─────────────
(call $vi.send            Linker::func_wrap          sys_send(dst,
  target ptr len)    →    "vi","send",           →   &mem[ptr..ptr+len])
                          |caller, dst, ptr, len| {
                            let mem = get_wasm_mem(&caller);
                            sys_send(dst, &mem[ptr..ptr+len])
                          }
```

---

## Related Code Files

### Modify
- `cells/drivers/wasm/src/lib.rs` — add `register_vi_imports(linker)` method

### Create
- `cells/drivers/wasm/src/imports.rs` — host function implementations

---

## Implementation Steps

### Step 1 — Create `imports.rs`

```rust
//! Host import functions exposed to WASM cells under the "vi" namespace.
//!
//! Each function receives a `Caller<HostState>` which provides access to the
//! WASM linear memory and the host state (cell_id).

use wasmi::{Caller, Linker};
use crate::HostState;

/// Register all `vi.*` host imports into the linker.
///
/// Call this before instantiating any WASM module that imports from "vi".
pub fn register_vi_imports(linker: &mut Linker<HostState>) {
    linker
        .func_wrap("vi", "send", vi_send)
        .expect("vi.send not already defined");
    linker
        .func_wrap("vi", "recv", vi_recv)
        .expect("vi.recv not already defined");
    linker
        .func_wrap("vi", "log", vi_log)
        .expect("vi.log not already defined");
    linker
        .func_wrap("vi", "exit", vi_exit)
        .expect("vi.exit not already defined");
}

/// `vi.send(target: i32, ptr: i32, len: i32) -> i32`
///
/// Reads `len` bytes at `ptr` from WASM linear memory and sends them to
/// `target` task via `sys_send`.  Returns 0 on success, -1 on invalid args.
fn vi_send(
    caller: Caller<'_, HostState>,
    target: i32,
    ptr: i32,
    len: i32,
) -> i32 {
    if len < 0 || ptr < 0 { return -1; }
    let Some(mem) = caller.get_export("memory")
        .and_then(|e| e.into_memory()) else { return -1; };
    let data = mem.data(&caller);
    let start = ptr as usize;
    let end = start.saturating_add(len as usize);
    if end > data.len() { return -1; }
    // SAFETY: data is a shared slice of WASM linear memory valid for this call.
    ostd::syscall::sys_send(target as usize, &data[start..end]);
    0
}

/// `vi.recv(ptr: i32, max_len: i32, sender_out: i32) -> i32`
///
/// Blocks until a message arrives, writes bytes into WASM memory at `ptr`,
/// writes the sender task-id as a little-endian i32 at `sender_out`.
/// Returns the number of bytes received, or -1 on error.
fn vi_recv(
    mut caller: Caller<'_, HostState>,
    ptr: i32,
    max_len: i32,
    sender_out: i32,
) -> i32 {
    if max_len <= 0 || ptr < 0 { return -1; }
    let Some(mem) = caller.get_export("memory")
        .and_then(|e| e.into_memory()) else { return -1; };

    let capacity = max_len as usize;
    let mut recv_buf = alloc::vec![0u8; capacity];

    match ostd::syscall::sys_recv(0, &mut recv_buf) {
        ostd::syscall::SyscallResult::Ok(sender) => {
            // ViCell sys_recv returns sender_id, not byte count.  The kernel fills
            // recv_buf with min(sender_len, capacity) bytes.  To avoid corrupting
            // binary payloads with legitimate trailing zeros (postcard-encoded data,
            // length-prefixed structs), use capacity as an upper bound and rely on
            // the postcard decoder in the WASM cell to determine the true message end.
            let n = capacity; // pass full capacity; WASM caller decodes with take_from_bytes
            let data = mem.data_mut(&mut caller);
            let start = ptr as usize;
            let end = start.saturating_add(n);
            if end > data.len() { return -1; }
            data[start..end].copy_from_slice(&recv_buf[..n]);
            // Write sender_id as LE i32 at sender_out
            let so = sender_out as usize;
            if so + 4 <= data.len() {
                data[so..so+4].copy_from_slice(&(sender as u32).to_le_bytes());
            }
            n as i32
        }
        _ => -1,
    }
}

/// `vi.log(ptr: i32, len: i32)` — write UTF-8 string to kernel serial log.
fn vi_log(caller: Caller<'_, HostState>, ptr: i32, len: i32) {
    if len <= 0 || ptr < 0 { return; }
    let Some(mem) = caller.get_export("memory")
        .and_then(|e| e.into_memory()) else { return; };
    let data = mem.data(&caller);
    let start = ptr as usize;
    let end = start.saturating_add(len as usize);
    if end > data.len() { return; }
    if let Ok(s) = core::str::from_utf8(&data[start..end]) {
        ostd::io::print(s);
    }
}

/// `vi.exit(code: i32)` — terminate the WASM cell cleanly.
fn vi_exit(_caller: Caller<'_, HostState>, code: i32) {
    ostd::syscall::sys_exit(code as usize);
}
```

### Step 2 — Add `register_vi_imports` to `WasmRuntime`

In `lib.rs`:
```rust
pub mod imports;

impl WasmRuntime {
    pub fn register_vi_imports(&self, linker: &mut wasmi::Linker<HostState>) {
        imports::register_vi_imports(linker);
    }
}
```

---

## Todo List

- [ ] Create `cells/drivers/wasm/src/imports.rs` with 4 host functions
- [ ] Add `pub mod imports;` to `lib.rs`; add `register_vi_imports` to `WasmRuntime`
- [ ] `cargo check --target riscv64gc-unknown-none-elf -Z build-std=core,alloc -p driver-wasm` — clean
- [ ] Unit test: load a tiny WASM module that calls `vi.log("hello")` — verify output

---

## Success Criteria

- [ ] `vi.send` correctly reads from WASM linear memory and calls `sys_send`
- [ ] `vi.recv` blocks until a message arrives (blocking `sys_recv` call)
- [ ] `vi.log` writes to serial console without kernel panic
- [ ] `vi.exit` terminates the WASM cell without halting the kernel
- [ ] Out-of-bounds WASM memory access returns -1 (not kernel panic)
