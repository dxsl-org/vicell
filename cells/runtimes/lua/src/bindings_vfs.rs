//! VFS filesystem bindings exposed to Lua via C FFI (`vfs.*`).
//!
//! Uses the typed postcard IPC (`api::ipc::VfsRequest/VfsResponse`) introduced
//! at Milestone 2.1.  The old raw byte-opcode protocol (OP_READ=8, OP_WRITE=4…)
//! was removed from the VFS cell and must not be used here.
//!
//! Reference pattern: `cells/tools/shell/src/cmd_fs.rs` (`vfs_req_ok`, `read_file_vfs`).
// `L` is the universal Lua C API convention for `lua_State*`.
#![allow(non_snake_case)] // reason: L is the Lua C API convention for lua_State pointers

extern crate alloc;

use core::ffi::{c_char, c_int};
use crate::ffi::{self, LuaState};

const VFS_ENDPOINT: usize = 3;
/// Safe payload size per IPC call: 512 byte frame minus postcard overhead and path length.
const MAX_CHUNK: usize = 400;

// ─── IPC helpers ──────────────────────────────────────────────────────────────

/// Send a typed VfsRequest to the VFS cell and return `true` when the reply is `Ok`.
pub fn vfs_ok(req: &api::ipc::VfsRequest<'_>) -> bool {
    let mut buf = [0u8; 512];
    let n = match api::ipc::encode(req, &mut buf) {
        Ok(s) => s.len(),
        Err(_) => return false,
    };
    ostd::syscall::sys_send(VFS_ENDPOINT, &buf[..n]);
    let mut reply = [0u8; 64];
    match ostd::syscall::sys_recv(0, &mut reply) {
        ostd::syscall::SyscallResult::Ok(_) => {
            matches!(
                api::ipc::decode::<api::ipc::VfsResponse>(&reply),
                Ok(api::ipc::VfsResponse::Ok)
            )
        }
        _ => false,
    }
}

/// Maximum file size read in a single `vfs_get_file_vec` call (64 KB).
///
/// Prevents OOM when the VFS returns an unexpectedly large `DataPtr.len`.
/// Larger files are silently capped; callers should `vfs.stat` first if size matters.
const MAX_FILE_READ: usize = 64 * 1024;

/// Read file content from VFS into `out` via `GetFile`. Returns bytes copied.
///
/// `GetFile` returns a `DataPtr` (zero-copy pointer into VFS SAS memory). The
/// bytes MUST be copied before the next `sys_recv` — VFS may reuse the buffer
/// once it processes the next request. Copies up to `out.len()` bytes.
///
/// For reads where the file size is unknown in advance, prefer `vfs_get_file_vec`.
pub fn vfs_get_file(path: &str, out: &mut [u8]) -> usize {
    let req = api::ipc::VfsRequest::GetFile(path);
    let mut buf = [0u8; 512];
    let n = match api::ipc::encode(&req, &mut buf) {
        Ok(s) => s.len(),
        Err(_) => return 0,
    };
    ostd::syscall::sys_send(VFS_ENDPOINT, &buf[..n]);
    let mut reply = [0u8; 512];
    match ostd::syscall::sys_recv(0, &mut reply) {
        ostd::syscall::SyscallResult::Ok(_) => {
            match api::ipc::decode::<api::ipc::VfsResponse>(&reply) {
                Ok(api::ipc::VfsResponse::DataPtr { ptr, len }) => {
                    let data_len = (len as usize).min(out.len());
                    // SAFETY: ptr is a valid SAS pointer from VFS; the VFS cell is
                    // blocked in its recv loop while we hold the reply, so the pointed
                    // memory is stable for the duration of this copy.
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            ptr as *const u8,
                            out.as_mut_ptr(),
                            data_len,
                        );
                    }
                    data_len
                }
                _ => 0,
            }
        }
        _ => 0,
    }
}

/// Read file content into a `Vec<u8>` sized from the `DataPtr.len` response.
///
/// Unlike `vfs_get_file`, this allocates exactly as many bytes as the VFS
/// reports — no silent truncation at a hardcoded buffer size.  Capped at
/// `MAX_FILE_READ` (64 KB) to prevent OOM on corrupt replies.
pub fn vfs_get_file_vec(path: &str) -> alloc::vec::Vec<u8> {
    let req = api::ipc::VfsRequest::GetFile(path);
    let mut buf = [0u8; 512];
    let n = match api::ipc::encode(&req, &mut buf) {
        Ok(s) => s.len(),
        Err(_) => return alloc::vec![],
    };
    ostd::syscall::sys_send(VFS_ENDPOINT, &buf[..n]);
    let mut reply = [0u8; 512];
    match ostd::syscall::sys_recv(0, &mut reply) {
        ostd::syscall::SyscallResult::Ok(_) => {
            match api::ipc::decode::<api::ipc::VfsResponse>(&reply) {
                Ok(api::ipc::VfsResponse::DataPtr { ptr, len }) => {
                    let size = (len as usize).min(MAX_FILE_READ);
                    let mut out = alloc::vec![0u8; size];
                    // SAFETY: ptr is a valid SAS pointer from VFS; copy before next sys_recv.
                    unsafe {
                        core::ptr::copy_nonoverlapping(ptr as *const u8, out.as_mut_ptr(), size);
                    }
                    out
                }
                _ => alloc::vec![],
            }
        }
        _ => alloc::vec![],
    }
}

/// Write raw bytes to `path` from Rust (not Lua). Used at startup to install
/// bundled scripts into `/tmp` so `require()` can find them.
pub fn write_bytes(path: &str, data: &[u8]) -> bool {
    vfs_write_chunked(path, data, false)
}

/// Write `data` to `path`, chunking into MAX_CHUNK-byte IPC payloads.
///
/// The first chunk uses `Write` (create/overwrite); subsequent chunks use `Append`.
/// When `append` is true every chunk uses `Append`.
fn vfs_write_chunked(path: &str, data: &[u8], append: bool) -> bool {
    if data.is_empty() {
        return if append {
            true
        } else {
            vfs_ok(&api::ipc::VfsRequest::Write { path, content: &[] })
        };
    }
    let mut first = !append;
    let mut ok = true;
    for chunk in data.chunks(MAX_CHUNK) {
        let req = if first {
            first = false;
            api::ipc::VfsRequest::Write { path, content: chunk }
        } else {
            api::ipc::VfsRequest::Append { path, content: chunk }
        };
        ok &= vfs_ok(&req);
    }
    ok
}

// ─── Lua argument helpers ─────────────────────────────────────────────────────

/// Read the string arg at stack `idx` as a byte slice borrowed from Lua.
///
/// # Safety
/// `L` must be valid; the slice lives only while the value stays on the Lua stack.
unsafe fn lua_arg_bytes<'a>(L: *mut LuaState, idx: c_int) -> Option<&'a [u8]> {
    let mut len: usize = 0;
    // SAFETY: caller guarantees L is valid; idx is a valid stack position.
    let ptr = unsafe { ffi::lua_tolstring(L, idx, &mut len as *mut _) };
    if ptr.is_null() { return None; }
    // SAFETY: Lua guarantees `len` valid bytes at `ptr`.
    Some(unsafe { core::slice::from_raw_parts(ptr as *const u8, len) })
}

/// Extract a path `&str` from Lua arg at `idx`. Returns `None` on error.
unsafe fn lua_arg_path<'a>(L: *mut LuaState, idx: c_int) -> Option<&'a str> {
    let raw = unsafe { lua_arg_bytes(L, idx) }?;
    core::str::from_utf8(raw).ok().filter(|s| !s.is_empty())
}

// ─── Core vfs.* Lua bindings ──────────────────────────────────────────────────

/// `vfs.read(path)` → string | nil
///
/// Reads file content from VFS. Returns the content as a Lua string, or nil if
/// the file is missing or empty. Allocates exactly as many bytes as the VFS
/// reports (up to 64 KB) so large files are not silently truncated.
#[no_mangle]
pub unsafe extern "C" fn vfs_read(L: *mut LuaState) -> c_int {
    let path = match unsafe { lua_arg_path(L, 1) } {
        Some(p) => p,
        None => { unsafe { ffi::lua_pushnil(L) }; return 1; }
    };
    let data = vfs_get_file_vec(path);
    if data.is_empty() { unsafe { ffi::lua_pushnil(L) }; return 1; }
    // SAFETY: L valid; data contains the initialised file content.
    unsafe { ffi::lua_pushlstring(L, data.as_ptr() as *const c_char, data.len()) };
    1
}

/// `vfs.write(path, content)` → bool
///
/// Creates or overwrites a file. Content larger than 400 bytes is split into
/// multiple Write+Append IPC calls.
#[no_mangle]
pub unsafe extern "C" fn vfs_write(L: *mut LuaState) -> c_int {
    let path = match unsafe { lua_arg_path(L, 1) } {
        Some(p) => p,
        None => { unsafe { ffi::lua_pushboolean(L, 0) }; return 1; }
    };
    let content = unsafe { lua_arg_bytes(L, 2) }.unwrap_or(&[]);
    let ok = vfs_write_chunked(path, content, false);
    unsafe { ffi::lua_pushboolean(L, if ok { 1 } else { 0 }) };
    1
}

/// `vfs.append(path, content)` → bool
///
/// Appends content to an existing file (or creates it).
#[no_mangle]
pub unsafe extern "C" fn vfs_append(L: *mut LuaState) -> c_int {
    let path = match unsafe { lua_arg_path(L, 1) } {
        Some(p) => p,
        None => { unsafe { ffi::lua_pushboolean(L, 0) }; return 1; }
    };
    let content = unsafe { lua_arg_bytes(L, 2) }.unwrap_or(&[]);
    let ok = vfs_write_chunked(path, content, true);
    unsafe { ffi::lua_pushboolean(L, if ok { 1 } else { 0 }) };
    1
}

/// `vfs.mkdir(path)` → bool
#[no_mangle]
pub unsafe extern "C" fn vfs_mkdir(L: *mut LuaState) -> c_int {
    let path = match unsafe { lua_arg_path(L, 1) } {
        Some(p) => p,
        None => { unsafe { ffi::lua_pushboolean(L, 0) }; return 1; }
    };
    let ok = vfs_ok(&api::ipc::VfsRequest::Mkdir(path));
    unsafe { ffi::lua_pushboolean(L, if ok { 1 } else { 0 }) };
    1
}

// ─── Extended vfs.* bindings (Phase 03) ───────────────────────────────────────

/// `vfs.stat(path)` → {size=N, is_dir=bool} | nil
///
/// Returns a table with file metadata, or nil if the path does not exist.
#[no_mangle]
pub unsafe extern "C" fn vfs_stat(L: *mut LuaState) -> c_int {
    let path = match unsafe { lua_arg_path(L, 1) } {
        Some(p) => p,
        None => { unsafe { ffi::lua_pushnil(L) }; return 1; }
    };
    let mut buf = [0u8; 512];
    let n = match api::ipc::encode(&api::ipc::VfsRequest::Stat(path), &mut buf) {
        Ok(s) => s.len(),
        Err(_) => { unsafe { ffi::lua_pushnil(L) }; return 1; }
    };
    ostd::syscall::sys_send(VFS_ENDPOINT, &buf[..n]);
    let mut reply = [0u8; 64];
    match ostd::syscall::sys_recv(0, &mut reply) {
        ostd::syscall::SyscallResult::Ok(_) => {
            match api::ipc::decode::<api::ipc::VfsResponse>(&reply) {
                Ok(api::ipc::VfsResponse::Stat { size, is_dir }) => {
                    // Build {size=N, is_dir=bool} table.
                    unsafe { ffi::lua_createtable(L, 0, 2) };
                    let t = unsafe { ffi::lua_gettop(L) };
                    unsafe { ffi::lua_pushinteger(L, size as i64) };
                    unsafe { ffi::lua_setfield(L, t, c"size".as_ptr()) };
                    unsafe { ffi::lua_pushboolean(L, if is_dir { 1 } else { 0 }) };
                    unsafe { ffi::lua_setfield(L, t, c"is_dir".as_ptr()) };
                    1
                }
                _ => { unsafe { ffi::lua_pushnil(L) }; 1 }
            }
        }
        _ => { unsafe { ffi::lua_pushnil(L) }; 1 }
    }
}

/// `vfs.listdir(path)` → array of "d:name" / "f:name" strings | nil
///
/// Returns a 1-indexed Lua array. Entries are prefixed with `d:` (directory)
/// or `f:` (file). Directories with more than ~30 entries are silently truncated
/// by the 512-byte VFS reply limit.
#[no_mangle]
pub unsafe extern "C" fn vfs_listdir(L: *mut LuaState) -> c_int {
    let path = match unsafe { lua_arg_path(L, 1) } {
        Some(p) => p,
        None => { unsafe { ffi::lua_pushnil(L) }; return 1; }
    };
    let mut buf = [0u8; 512];
    let n = match api::ipc::encode(&api::ipc::VfsRequest::ListDir(path), &mut buf) {
        Ok(s) => s.len(),
        Err(_) => { unsafe { ffi::lua_pushnil(L) }; return 1; }
    };
    ostd::syscall::sys_send(VFS_ENDPOINT, &buf[..n]);
    let mut reply = [0u8; 512];
    match ostd::syscall::sys_recv(0, &mut reply) {
        ostd::syscall::SyscallResult::Ok(_) => {
            match api::ipc::decode::<api::ipc::VfsResponse>(&reply) {
                Ok(api::ipc::VfsResponse::Data(entries)) => {
                    let text = core::str::from_utf8(entries).unwrap_or("");
                    unsafe { ffi::lua_createtable(L, 0, 0) };
                    let t = unsafe { ffi::lua_gettop(L) };
                    let mut i = 1i64;
                    for line in text.lines() {
                        if line.is_empty() { continue; }
                        // SAFETY: line borrows from `reply` which is alive; push copies.
                        unsafe {
                            ffi::lua_pushlstring(L, line.as_ptr() as *const c_char, line.len());
                            ffi::lua_rawseti(L, t, i);
                        }
                        i += 1;
                    }
                    1
                }
                _ => { unsafe { ffi::lua_pushnil(L) }; 1 }
            }
        }
        _ => { unsafe { ffi::lua_pushnil(L) }; 1 }
    }
}

/// `vfs.remove(path)` → bool
///
/// Deletes a file from VFS. Returns false if the file does not exist.
#[no_mangle]
pub unsafe extern "C" fn vfs_remove(L: *mut LuaState) -> c_int {
    let path = match unsafe { lua_arg_path(L, 1) } {
        Some(p) => p,
        None => { unsafe { ffi::lua_pushboolean(L, 0) }; return 1; }
    };
    let ok = vfs_ok(&api::ipc::VfsRequest::Unlink(path));
    unsafe { ffi::lua_pushboolean(L, if ok { 1 } else { 0 }) };
    1
}
