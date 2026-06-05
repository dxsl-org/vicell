//! Host import functions exposed to WASM cells under the `vi` namespace.
//!
//! Each function receives a `Caller<HostState>` giving access to the WASM
//! linear memory and the host state (cell task ID).  Register them into
//! a `Linker` before instantiating any module that imports from `vi`.

use wasmi::{Caller, Linker};
use crate::HostState;

/// Register all `vi.*` host imports into `linker`.
///
/// Must be called before `linker.instantiate_and_start`.
pub fn register_vi_imports(linker: &mut Linker<HostState>) {
    linker.func_wrap("vi", "send", vi_send).expect("vi.send");
    linker.func_wrap("vi", "recv", vi_recv).expect("vi.recv");
    linker.func_wrap("vi", "log",  vi_log).expect("vi.log");
    linker.func_wrap("vi", "exit", vi_exit).expect("vi.exit");
}

// ── vi.send ───────────────────────────────────────────────────────────────────

/// `vi.send(target: i32, ptr: i32, len: i32) → i32`
///
/// Reads `len` bytes at `ptr` from WASM linear memory and sends them
/// to kernel task `target` via `sys_send`.  Returns 0 on success, -1
/// on invalid arguments (negative values, out-of-bounds ptr+len).
fn vi_send(caller: Caller<'_, HostState>, target: i32, ptr: i32, len: i32) -> i32 {
    if target < 0 || ptr < 0 || len < 0 { return -1; }
    let mem = match caller.get_export("memory").and_then(|e| e.into_memory()) {
        Some(m) => m,
        None => return -1,
    };
    let data = mem.data(&caller);
    let start = ptr as usize;
    let end = start.saturating_add(len as usize);
    if end > data.len() { return -1; }
    ostd::syscall::sys_send(target as usize, &data[start..end]);
    0
}

// ── vi.recv ───────────────────────────────────────────────────────────────────

/// `vi.recv(ptr: i32, max_len: i32, sender_out: i32) → i32`
///
/// Blocks until a message arrives, copies bytes into WASM memory at `ptr`
/// (up to `max_len` bytes), and writes the sender task-id as a LE u32 at
/// `sender_out`.  Returns `max_len` (full-capacity slice; WASM caller should
/// use postcard `take_from_bytes` to find the true message boundary).
///
/// Returns -1 on invalid arguments or receive error.
fn vi_recv(
    mut caller: Caller<'_, HostState>,
    ptr: i32,
    max_len: i32,
    sender_out: i32,
) -> i32 {
    if ptr < 0 || max_len <= 0 { return -1; }
    let capacity = max_len as usize;
    let mut recv_buf = alloc::vec![0u8; capacity];

    // sys_recv always returns SyscallResult::Ok(sender_id) — the Err arm is
    // unreachable with the current ostd API.  sender==0 is a proxy for error
    // (kernel task; no real cell sends from ID 0 in normal operation).
    match ostd::syscall::sys_recv(0, &mut recv_buf) {
        ostd::syscall::SyscallResult::Ok(sender) if sender > 0 => {
            let mem = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                Some(m) => m,
                None => return -1,
            };
            // Copy full capacity — WASM caller uses postcard to find message end.
            // sys_recv fills min(sender_len, capacity) bytes; remainder is zero.
            {
                let data = mem.data_mut(&mut caller);
                let start = ptr as usize;
                let end = start.saturating_add(capacity);
                if end > data.len() { return -1; }
                data[start..end].copy_from_slice(&recv_buf);
            }
            // Write sender task-id as LE u32 at sender_out.
            if sender_out >= 0 {
                let data = mem.data_mut(&mut caller);
                let so = sender_out as usize;
                if so + 4 <= data.len() {
                    data[so..so + 4].copy_from_slice(&(sender as u32).to_le_bytes());
                }
            }
            capacity as i32
        }
        _ => -1,
    }
}

// ── vi.log ────────────────────────────────────────────────────────────────────

/// `vi.log(ptr: i32, len: i32)`
///
/// Writes a UTF-8 string from WASM linear memory to the kernel serial log.
/// Silently ignores invalid UTF-8 or out-of-bounds pointers.
fn vi_log(caller: Caller<'_, HostState>, ptr: i32, len: i32) {
    if ptr < 0 || len <= 0 { return; }
    let mem = match caller.get_export("memory").and_then(|e| e.into_memory()) {
        Some(m) => m,
        None => return,
    };
    let data = mem.data(&caller);
    let start = ptr as usize;
    let end = start.saturating_add(len as usize);
    if end > data.len() { return; }
    if let Ok(s) = core::str::from_utf8(&data[start..end]) {
        ostd::io::print(s);
    }
}

// ── vi.exit ───────────────────────────────────────────────────────────────────

/// `vi.exit(code: i32)`
///
/// Terminates the WASM cell cleanly by calling `sys_exit`.
/// This host function never returns — the kernel transitions the task
/// to zombie state and the scheduler picks the next ready task.
fn vi_exit(_caller: Caller<'_, HostState>, code: i32) {
    ostd::syscall::sys_exit(code as usize);
}
