//! VFS filesystem bindings exposed to Lua via C FFI (`vfs.*`).
//!
//! Mirrors the verified IPC wire format used by cells/apps/shell/src/cmd_fs.rs.
//! Messages go to the VFS service cell (endpoint 3). sys_recv returns the SENDER
//! id, not a byte count — reply length is bounded by the buffer we pass.
// `L` is the universal Lua C API convention for `lua_State*`.
#![allow(non_snake_case)] // reason: L is the Lua C API convention for lua_State pointers

extern crate alloc;

use core::ffi::{c_char, c_int};
use crate::ffi::{self, LuaState};
use ostd::syscall::{sys_recv, sys_send, SyscallResult};

const VFS_ENDPOINT: usize = 3; // cmd_fs.rs:15
const OP_WRITE:  u8 = 4;       // cmd_fs.rs:279
const OP_MKDIR:  u8 = 5;       // cmd_fs.rs:16
const OP_READ:   u8 = 8;       // cmd_fs.rs:280
const OP_APPEND: u8 = 10;      // cmd_fs.rs:20
/// Conservative per-IPC content cap — subset of the shell's working `512 - 4 - path_len`.
const MAX_CHUNK: usize = 480;

/// Read the string arg at stack `idx` as a byte slice borrowed from Lua.
///
/// # Safety
/// `L` must be valid; the slice lives only while the value stays on the Lua stack.
unsafe fn lua_arg_bytes<'a>(L: *mut LuaState, idx: c_int) -> Option<&'a [u8]> {
    let mut len: usize = 0;
    // SAFETY: caller guarantees L is valid; idx is a checked stack position.
    let ptr = unsafe { ffi::lua_tolstring(L, idx, &mut len as *mut _) };
    if ptr.is_null() { return None; }
    // SAFETY: Lua guarantees `len` valid bytes at `ptr`.
    Some(unsafe { core::slice::from_raw_parts(ptr as *const u8, len) })
}

/// `vfs.read(path)` → string | nil
///
/// Reads file content from VFS via OP_READ. Returns nil if the file is missing
/// or empty. Uses zero-scan to detect reply length (sys_recv returns sender_id).
#[no_mangle]
pub unsafe extern "C" fn vfs_read(L: *mut LuaState) -> c_int {
    // SAFETY: L valid; arg 1 is the path string.
    let raw = match unsafe { lua_arg_bytes(L, 1) } {
        Some(b) => b,
        None => { unsafe { ffi::lua_pushnil(L) }; return 1; }
    };
    let path = match core::str::from_utf8(raw) {
        Ok(s) if !s.is_empty() => s,
        _ => { unsafe { ffi::lua_pushnil(L) }; return 1; }
    };
    let pb = path.as_bytes();
    let pl = pb.len().min(253) as u8;
    let mut req = [0u8; 256];
    req[0] = OP_READ;
    req[1] = pl;
    req[2..2 + pl as usize].copy_from_slice(&pb[..pl as usize]);
    sys_send(VFS_ENDPOINT, &req[..2 + pl as usize]);
    let mut buf = alloc::vec![0u8; 4096];
    match sys_recv(0, &mut buf) {
        SyscallResult::Ok(_) => {
            let n = buf.iter().rposition(|&b| b != 0).map(|i| i + 1).unwrap_or(0);
            if n == 0 { unsafe { ffi::lua_pushnil(L) }; return 1; }
            // SAFETY: L valid; buf[..n] is initialised file content bytes.
            unsafe { ffi::lua_pushlstring(L, buf.as_ptr() as *const c_char, n) };
            1
        }
        _ => { unsafe { ffi::lua_pushnil(L) }; 1 }
    }
}

/// `vfs.write(path, content)` → bool
///
/// Writes content to path via OP_WRITE. Content >480 bytes is split into
/// OP_WRITE + OP_APPEND chunks to stay within the VFS cell's receive budget.
#[no_mangle]
pub unsafe extern "C" fn vfs_write(L: *mut LuaState) -> c_int {
    vfs_write_impl(L, OP_WRITE)
}

/// `vfs.append(path, content)` → bool
///
/// Appends content to path via OP_APPEND chunks.
#[no_mangle]
pub unsafe extern "C" fn vfs_append(L: *mut LuaState) -> c_int {
    vfs_write_impl(L, OP_APPEND)
}

/// Shared write/append driver.
///
/// `first_op` is OP_WRITE (truncate+write) or OP_APPEND. Content larger than
/// MAX_CHUNK is split: the first chunk uses `first_op`, subsequent chunks use
/// OP_APPEND.  `max_chunk.max(1)` prevents an infinite loop when path_len is near 480.
fn vfs_write_impl(L: *mut LuaState, first_op: u8) -> c_int {
    // SAFETY: L valid; args 1 and 2 stay on the Lua stack throughout this fn.
    let pb = match unsafe { lua_arg_bytes(L, 1) } {
        Some(b) => b,
        None => { unsafe { ffi::lua_pushboolean(L, 0) }; return 1; }
    };
    let content = unsafe { lua_arg_bytes(L, 2) }.unwrap_or(&[]);
    let pl = pb.len().min(253);
    let max_chunk = MAX_CHUNK.saturating_sub(pl).max(1);
    let first_len = content.len().min(max_chunk);
    let mut ok = vfs_op_write_chunk(first_op, pb, &content[..first_len]);
    let mut offset = first_len;
    while ok && offset < content.len() {
        let end = (offset + max_chunk).min(content.len());
        ok = vfs_op_write_chunk(OP_APPEND, pb, &content[offset..end]);
        offset = end;
    }
    // SAFETY: L valid.
    unsafe { ffi::lua_pushboolean(L, if ok { 1 } else { 0 }) };
    1
}

/// `vfs.mkdir(path)` → bool
#[no_mangle]
pub unsafe extern "C" fn vfs_mkdir(L: *mut LuaState) -> c_int {
    // SAFETY: L valid; arg 1 is the path string.
    let pb = match unsafe { lua_arg_bytes(L, 1) } {
        Some(b) => b,
        None => { unsafe { ffi::lua_pushboolean(L, 0) }; return 1; }
    };
    let pl = pb.len().min(253);
    let mut req = [0u8; 256];
    req[0] = OP_MKDIR;
    req[1] = pl as u8;
    req[2..2 + pl].copy_from_slice(&pb[..pl]);
    sys_send(VFS_ENDPOINT, &req[..2 + pl]);
    let mut r = [0u8; 1];
    let ok = match sys_recv(0, &mut r) {
        SyscallResult::Ok(_) => r[0] == 0,
        _ => false,
    };
    // SAFETY: L valid.
    unsafe { ffi::lua_pushboolean(L, if ok { 1 } else { 0 }) };
    1
}

/// Send one OP_WRITE/OP_APPEND IPC chunk to the VFS cell. Returns true on success.
///
/// Mirrors `write_file` in cells/apps/shell/src/cmd_fs.rs:285.
fn vfs_op_write_chunk(opcode: u8, path: &[u8], content: &[u8]) -> bool {
    let pl = path.len().min(253);
    let cl = content.len().min(MAX_CHUNK.saturating_sub(pl));
    let mut buf = alloc::vec![0u8; 4 + pl + cl];
    buf[0] = opcode;
    buf[1] = pl as u8;
    buf[2..4].copy_from_slice(&(cl as u16).to_le_bytes());
    buf[4..4 + pl].copy_from_slice(&path[..pl]);
    buf[4 + pl..4 + pl + cl].copy_from_slice(&content[..cl]);
    sys_send(VFS_ENDPOINT, &buf);
    let mut r = [0u8; 1];
    match sys_recv(0, &mut r) {
        SyscallResult::Ok(_) => r[0] == 0,
        _ => false,
    }
}
