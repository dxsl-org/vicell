#![no_std]
#![no_main]

extern crate ostd;
extern crate api;

mod ffi;

use core::ffi::c_char;
use ffi::{LuaState, LUA_OK};

/// Read until `\n` or EOF from stdin into a fixed buffer.
/// Returns the number of bytes read (including the newline if present).
fn read_stdin(buf: &mut [u8]) -> usize {
    let mut n = 0;
    while n < buf.len() {
        let mut c = [0u8; 1];
        match ostd::syscall::sys_read(0, &mut c) {
            Ok(1) => {
                buf[n] = c[0];
                n += 1;
                if c[0] == b'\n' { break; }
            }
            _ => break,
        }
    }
    n
}

/// Run `code` as a Lua chunk on `L`.  Print any error to stderr.
///
/// Returns `true` on success, `false` on error.
///
/// # Safety
/// `L` must be a valid, non-null Lua state.
#[allow(non_snake_case)] // reason: L is the Lua API convention for lua_State pointers
unsafe fn eval(L: *mut LuaState, code: &[u8]) -> bool {
    // Ensure code is NUL-terminated.
    let mut nul_buf: [u8; 8192] = [0; 8192];
    let len = code.len().min(nul_buf.len() - 1);
    nul_buf[..len].copy_from_slice(&code[..len]);

    // SAFETY: nul_buf contains valid UTF-8 bytes followed by a NUL terminator.
    let rc = unsafe { ffi::luaL_loadstring(L, nul_buf.as_ptr() as *const c_char) };
    if rc != LUA_OK {
        // SAFETY: Lua pushed an error string; tolstring is valid here.
        let err = unsafe { ffi::lua_tolstring(L, -1, core::ptr::null_mut()) };
        if !err.is_null() {
            let msg = unsafe { core::ffi::CStr::from_ptr(err).to_bytes() };
            ostd::io::print("[lua] compile error: ");
            if let Ok(s) = core::str::from_utf8(msg) { ostd::io::println(s); }
        }
        return false;
    }

    let rc = unsafe { ffi::lua_pcall(L, 0, ffi::LUA_MULTRET, 0) };
    if rc != LUA_OK {
        let err = unsafe { ffi::lua_tolstring(L, -1, core::ptr::null_mut()) };
        if !err.is_null() {
            let msg = unsafe { core::ffi::CStr::from_ptr(err).to_bytes() };
            ostd::io::print("[lua] runtime error: ");
            if let Ok(s) = core::str::from_utf8(msg) { ostd::io::println(s); }
        }
        return false;
    }
    true
}

#[no_mangle]
#[allow(non_snake_case)] // reason: L is the Lua API convention
extern "C" fn main() -> usize {
    // SAFETY: luaL_newstate allocates a new Lua state via malloc;
    // the returned pointer is valid until lua_close is called.
    let L = unsafe { ffi::luaL_newstate() };
    if L.is_null() {
        ostd::io::println("[lua] out of memory");
        return 1;
    }

    // SAFETY: L is non-null; luaL_openlibs is safe to call once.
    unsafe { ffi::luaL_openlibs(L) };

    // Simple REPL: read a line, eval it.
    ostd::io::println("Lua 5.4 on ViOS");
    let mut buf = [0u8; 4096];
    loop {
        ostd::io::print("> ");
        let n = read_stdin(&mut buf);
        if n == 0 { break; }

        // SAFETY: L is non-null and valid.
        unsafe { eval(L, &buf[..n]); }

        // Pop any remaining stack items from the last expression.
        let top = unsafe { ffi::lua_gettop(L) };
        if top > 0 { unsafe { ffi::lua_pop(L, top) }; }
    }

    // SAFETY: L is non-null; lua_close frees the state.
    unsafe { ffi::lua_close(L) };
    0
}
