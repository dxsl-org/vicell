//! Lua 5.4 C FFI bindings.
//!
//! Only the minimal subset needed by the ViOS Lua cell.  See lua.h and
//! lauxlib.h in `src/c/src/` for the full API.

use core::ffi::{c_char, c_int};

/// Opaque Lua interpreter state.  Always heap-allocated by Lua; never moved.
#[repr(C)]
pub struct LuaState {
    _opaque: [u8; 0],
}

pub const LUA_OK:      c_int = 0;
pub const LUA_MULTRET: c_int = -1;

extern "C" {
    // ── Lifecycle ────────────────────────────────────────────────────────────

    /// Create a new Lua state using Lua's default allocator.
    pub fn luaL_newstate() -> *mut LuaState;

    /// Open all standard libraries into `L`.
    pub fn luaL_openlibs(L: *mut LuaState);

    /// Close and free a Lua state.
    pub fn lua_close(L: *mut LuaState);

    // ── Execution ────────────────────────────────────────────────────────────

    /// Compile and push `s` as a Lua chunk onto the stack.
    /// Returns `LUA_OK` on success; otherwise pushes an error string.
    pub fn luaL_loadstring(L: *mut LuaState, s: *const c_char) -> c_int;

    /// Call the function at the top of the stack with `nargs` arguments.
    /// `LUA_MULTRET` for `nresults` accepts any number of results.
    ///
    /// Note: `lua_pcall` is a macro in lua.h that forwards to `lua_pcallk`
    /// with no continuation; we bind the real `lua_pcallk` symbol directly.
    /// `ctx`/`k` are the continuation context/function — pass 0/null for a
    /// plain protected call.
    pub fn lua_pcallk(
        L: *mut LuaState,
        nargs: c_int,
        nresults: c_int,
        errfunc: c_int,
        ctx: isize,
        k: *mut core::ffi::c_void,
    ) -> c_int;

    // ── Stack inspection ─────────────────────────────────────────────────────

    /// Return the string at stack index `idx` (negative = from top).
    /// Sets `*len` to the byte length.  Returns NULL if not a string.
    pub fn lua_tolstring(
        L: *mut LuaState,
        idx: c_int,
        len: *mut usize,
    ) -> *const c_char;

    /// Return the number of elements on the stack.
    pub fn lua_gettop(L: *mut LuaState) -> c_int;

    /// Pop `n` elements from the stack.
    pub fn lua_settop(L: *mut LuaState, idx: c_int);

    // ── Stack push ───────────────────────────────────────────────────────────

    /// Push a nil value.
    pub fn lua_pushnil(L: *mut LuaState);
    /// Push a boolean (0 = false, non-zero = true).
    pub fn lua_pushboolean(L: *mut LuaState, b: c_int);
    /// Push a 64-bit integer.
    pub fn lua_pushinteger(L: *mut LuaState, n: i64);
    /// Push a NUL-terminated C string; returns the internal pointer.
    pub fn lua_pushstring(L: *mut LuaState, s: *const c_char) -> *const c_char;
    /// Push `len` bytes of string data (may contain NUL bytes).
    pub fn lua_pushlstring(L: *mut LuaState, s: *const c_char, len: usize) -> *const c_char;
    /// Push a light userdata (opaque pointer, not GC-managed).
    pub fn lua_pushlightuserdata(L: *mut LuaState, p: *mut core::ffi::c_void);

    // ── Stack read ───────────────────────────────────────────────────────────

    /// Return the integer at stack index `idx` (0 if not a number).
    pub fn lua_tointegerx(L: *mut LuaState, idx: c_int, isnum: *mut c_int) -> i64;
    /// Return the light userdata pointer at stack index `idx` (null if wrong type).
    pub fn lua_touserdata(L: *mut LuaState, idx: c_int) -> *mut core::ffi::c_void;
}

/// Convenience: pop `n` items from the stack.
///
/// # Safety
/// `L` must be a valid, non-null Lua state.
#[allow(non_snake_case)] // reason: L is the Lua C API convention for lua_State pointers
#[allow(dead_code)] // reason: called by repl_session and bindings_io when stack cleanup is needed
pub unsafe fn lua_pop(L: *mut LuaState, n: c_int) {
    // SAFETY: caller guarantees L is valid; settop(-n-1) is the canonical pop.
    unsafe { lua_settop(L, -(n) - 1) }
}
