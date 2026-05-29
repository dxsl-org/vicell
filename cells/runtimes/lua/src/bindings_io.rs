//! Rust-side VFS I/O bindings exposed to Lua via C FFI.
// `L` is the universal Lua C API convention for `lua_State*` — suppress snake_case lint.
#![allow(non_snake_case)] // reason: L is the Lua C API convention for lua_State pointers
// ViosFile is part of the planned file-handle API; not yet wired into Lua's metatable.
#![allow(dead_code)] // reason: ViosFile and helper fns wired when io.open metatable lands
//!
//! Lua's `io.*` and `os.execute` are registered as C closures backed by these
//! Rust functions.  Since this module uses FFI it cannot be `#![forbid(unsafe_code)]`.
//!
//! Function signatures match what Lua expects via `lua_CFunction`:
//!   `extern "C" fn f(L: *mut lua_State) -> c_int`
//!
//! Each function pushes its return values and returns the count.

extern crate alloc;

use core::ffi::{c_char, c_int};
use crate::ffi::LuaState;

/// Maximum path length for VFS calls.
const MAX_PATH: usize = 256;
/// Maximum bytes for os.execute command string.
const MAX_CMD: usize = 512;

// ─── C-callable helpers ────────────────────────────────────────────────────

/// Read the string at stack position `idx` into a fixed buffer.
///
/// Returns `None` if the value is not a string or exceeds `MAX_PATH`.
unsafe fn lua_to_str<'a>(L: *mut LuaState, idx: c_int, buf: &'a mut [u8]) -> Option<&'a str> {
    let mut len: usize = 0;
    // SAFETY: caller ensures L is non-null; idx is a valid stack position.
    let ptr = unsafe { crate::ffi::lua_tolstring(L, idx, &mut len as *mut _) };
    if ptr.is_null() || len >= buf.len() { return None; }
    // SAFETY: lua guarantees `ptr` points to `len` valid bytes.
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
    buf[..len].copy_from_slice(bytes);
    core::str::from_utf8(&buf[..len]).ok()
}

// ─── os.execute binding ───────────────────────────────────────────────────

/// `os.execute(cmd)` — spawn `cmd` as an ELF binary via `SpawnFromPath`.
///
/// If `cmd` looks like an absolute path (`/bin/foo`) it is spawned directly.
/// Otherwise we prepend `/bin/` to find the binary.  Arguments are not yet
/// supported (Phase 17a); the binary is spawned with no args.
///
/// Returns exit code as an integer (0 = success, 1 = spawn failed).
/// Returns `true` with no arg to indicate a shell is available.
#[no_mangle]
pub unsafe extern "C" fn vios_os_execute(L: *mut LuaState) -> c_int {
    let mut path_buf = [0u8; MAX_CMD];
    // SAFETY: caller (Lua VM) provides a valid, initialised state.
    let cmd = unsafe { lua_to_str(L, 1, &mut path_buf) }.unwrap_or("");
    if cmd.is_empty() {
        // os.execute() with no arg returns true if a shell is available.
        // SAFETY: L is non-null; pushboolean is safe.
        unsafe { crate::ffi::lua_pushboolean(L, 1) };
        return 1;
    }

    // Resolve path: absolute paths used as-is; bare names prefixed with /bin/.
    let mut resolved = alloc::string::String::new();
    let path = if cmd.starts_with('/') {
        cmd
    } else {
        resolved.push_str("/bin/");
        resolved.push_str(cmd.split_whitespace().next().unwrap_or(cmd));
        resolved.as_str()
    };

    // Spawn the binary.  Wait semantics are not yet implemented — SpawnFromPath
    // enqueues the task; we return immediately with exit code 0 on success.
    let exit_code = match ostd::syscall::sys_spawn_from_path(path) {
        ostd::syscall::SyscallResult::Ok(_tid) => 0i64,
        ostd::syscall::SyscallResult::Err(_) => {
            ostd::io::print("[lua] os.execute: failed to spawn '");
            ostd::io::print(path);
            ostd::io::println("'");
            1i64
        }
    };
    // SAFETY: L is non-null.
    unsafe { crate::ffi::lua_pushinteger(L, exit_code) };
    1
}

// ─── io.open binding ──────────────────────────────────────────────────────

/// Light file-descriptor wrapper pushed as a Lua userdata.
pub struct ViosFile {
    pub fd: usize,
}

/// `io.open(path [, mode])` — open a VFS file and return a file handle.
///
/// Only "r" (read) is supported in v1.0.  Returns `nil, errmsg` on failure.
#[no_mangle]
pub unsafe extern "C" fn vios_io_open(L: *mut LuaState) -> c_int {
    let mut path_buf = [0u8; MAX_PATH];
    // SAFETY: L is non-null; stack index 1 holds the path argument.
    let path = match unsafe { lua_to_str(L, 1, &mut path_buf) } {
        Some(p) => p,
        None => {
            // SAFETY: L is non-null.
            unsafe { crate::ffi::lua_pushnil(L) };
            unsafe { crate::ffi::lua_pushstring(L, c"invalid path".as_ptr()) };
            return 2;
        }
    };

    match ostd::syscall::sys_open(path) {
        Ok(fd) => {
            // Push a lightuserdata representing the fd.
            // SAFETY: L is non-null; fd fits in a pointer-size integer.
            unsafe { crate::ffi::lua_pushlightuserdata(L, fd as *mut core::ffi::c_void) };
            1
        }
        Err(_) => {
            // SAFETY: L is non-null.
            unsafe { crate::ffi::lua_pushnil(L) };
            unsafe { crate::ffi::lua_pushstring(L, c"cannot open file".as_ptr()) };
            2
        }
    }
}

/// `io.read(fd, n)` / `handle:read(n)` — read up to `n` bytes from open fd.
#[no_mangle]
pub unsafe extern "C" fn vios_io_read(L: *mut LuaState) -> c_int {
    // SAFETY: L is non-null; stack index 1 = lightuserdata (fd).
    let fd = unsafe { crate::ffi::lua_touserdata(L, 1) } as usize;
    let n = (unsafe { crate::ffi::lua_tointegerx(L, 2, core::ptr::null_mut()) } as usize).min(4096);

    let mut buf = alloc::vec![0u8; n];
    match ostd::syscall::sys_read(fd, &mut buf) {
        Ok(0) => {
            // SAFETY: L is non-null; nil signals EOF.
            unsafe { crate::ffi::lua_pushnil(L) };
        }
        Ok(got) => {
            // SAFETY: L is non-null; buf has at least `got` valid bytes.
            unsafe {
                crate::ffi::lua_pushlstring(L, buf.as_ptr() as *const c_char, got);
            }
        }
        Err(_) => {
            unsafe { crate::ffi::lua_pushnil(L) };
        }
    }
    1
}

/// `io.close(fd)` — close an open file descriptor.
#[no_mangle]
pub unsafe extern "C" fn vios_io_close(L: *mut LuaState) -> c_int {
    // SAFETY: L is non-null; stack index 1 = lightuserdata (fd).
    let fd = unsafe { crate::ffi::lua_touserdata(L, 1) } as usize;
    ostd::syscall::sys_close(fd);
    // SAFETY: L is non-null.
    unsafe { crate::ffi::lua_pushboolean(L, 1) };
    1
}
