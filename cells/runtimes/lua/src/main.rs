#![no_std]
#![no_main]

extern crate alloc;
extern crate ostd;
extern crate api;

// Lua Cell: no direct network access — net data goes via IPC to the net Cell.
// Scripts load from VFS only; io.popen/os.execute/debug are stripped at init.
api::declare_manifest!(block_io = false, network = false, spawn = false);
api::declare_syscalls![Send, Recv, Log, Heartbeat];

// When no ELF-capable C compiler is available (e.g. Windows + MSVC without
// clang installed), build.rs emits `lua_c_unavailable` and the real Lua C
// library is not compiled. The cell still links as a stub that prints an
// informative error rather than silently crashing at link time.
#[cfg(lua_c_unavailable)]
#[no_mangle]
extern "C" fn main() -> usize {
    ostd::io::println("[lua] not available: compiled without an ELF C compiler.");
    ostd::io::println("[lua] Install LLVM/Clang and rebuild with a cross-compiler.");
    1
}

#[cfg(not(lua_c_unavailable))]
mod bindings_io;
#[cfg(not(lua_c_unavailable))]
mod bindings_vfs;
#[cfg(not(lua_c_unavailable))]
mod ffi;
#[cfg(not(lua_c_unavailable))]
mod repl_session;

// Bundled Lua scripts embedded at compile time; installed to /tmp at startup.
// Scripts are sourced from cells/runtimes/lua/scripts/ (MIT-licensed libraries
// and ViCell-authored tests).
#[cfg(not(lua_c_unavailable))]
const JSON_LUA:        &[u8] = include_bytes!("../scripts/json.lua");
#[cfg(not(lua_c_unavailable))]
const JSON_TEST_LUA:   &[u8] = include_bytes!("../scripts/json_test.lua");
#[cfg(not(lua_c_unavailable))]
const CORO_TEST_LUA:   &[u8] = include_bytes!("../scripts/coroutine_test.lua");

/// Install bundled Lua scripts into `/tmp` so `require()` can find them.
///
/// Called once at cell startup before the Lua state is created.  Overwrites
/// any stale copies from a previous run (idempotent — /tmp is RamFS).
/// Silent on VFS error: if VFS is not yet ready the scripts will simply be
/// missing (unlikely; VFS starts before the shell spawns Lua).
#[cfg(not(lua_c_unavailable))]
fn install_bundled_scripts() {
    bindings_vfs::write_bytes("/tmp/json.lua",            JSON_LUA);
    bindings_vfs::write_bytes("/tmp/json_test.lua",       JSON_TEST_LUA);
    bindings_vfs::write_bytes("/tmp/coroutine_test.lua",  CORO_TEST_LUA);
}

#[cfg(not(lua_c_unavailable))]
/// Read file content from VFS into an owned `Vec<u8>`.
///
/// Uses `vfs_get_file_vec` so the buffer is sized to the actual file length
/// (up to 64 KB), avoiding silent truncation at a fixed 4096-byte cap.
fn vfs_read_to_vec(path: &str) -> alloc::vec::Vec<u8> {
    bindings_vfs::vfs_get_file_vec(path)
}

#[cfg(not(lua_c_unavailable))]
/// Inject Lua-level `io.open` and `io.write` wrappers, then strip dangerous stdlib.
///
/// `ViCell_io_write` (C primitive) and `vfs.*` must already be registered.
/// Removes io.popen, os.execute, and debug to enforce the no-network policy.
///
/// # Safety
/// `L` must be a valid, non-null Lua state with `vfs` and `ViCell_io_write`
/// already registered as globals.
#[allow(non_snake_case)] // reason: L is the Lua C API convention for lua_State pointers
unsafe fn inject_io_setup(L: *mut ffi::LuaState) {
    // Single-quoted strings throughout to avoid escaping double-quotes in Rust.
    // string.char(10) produces the newline character for line-splitting.
    // Dangerous stdlib is nil'd here to enforce the sandbox boundary:
    //   io.popen  — would spawn a shell process (impossible in SAS, but remove anyway)
    //   os.execute — arbitrary shell commands
    //   debug     — can inspect/mutate internal Lua state, breaks sandbox
    //   package.loadlib — loads native C libraries (impossible without dlopen, remove anyway)
    const SETUP: &[u8] = b"
io.write = function(...)
  for _, v in ipairs({...}) do ViCell_io_write(tostring(v)) end
end
io.popen   = nil
os.execute = nil
debug      = nil
if package then
  package.loadlib = nil
  -- Override the file-system searcher to use VFS-backed io.open instead of
  -- C fopen (which calls ViSyscall::Open, not in this cell's allowlist).
  -- Scripts are installed to /tmp at startup via install_bundled_scripts().
  package.path = '/tmp/?.lua'
  package.searchers = {
    package.searchers[1],
    function(name)
      local path = '/tmp/' .. name:gsub('%.', '/') .. '.lua'
      local f = io.open(path, 'r')
      if not f then return 'module ' .. name .. ' not found at ' .. path end
      local src = f:read('*a')
      f:close()
      return load(src, '@' .. path)
    end,
  }
end
io.open = function(path, mode)
  mode = mode or 'r'
  if mode == 'r' or mode == 'rb' then
    local d = vfs.read(path)
    if d == nil then return nil, 'cannot open: ' .. tostring(path) end
    local h = {_d = d, _p = 1}
    local NL = string.char(10)
    function h:read(f)
      f = f or '*l'
      if f == '*a' or f == '*all' then
        self._p = #self._d + 1
        return self._d
      end
      local s, e = string.find(self._d, NL, self._p, true)
      if s then
        local line = string.sub(self._d, self._p, s - 1)
        self._p = e + 1
        return line
      elseif self._p <= #self._d then
        local line = string.sub(self._d, self._p)
        self._p = #self._d + 1
        return line
      end
      return nil
    end
    function h:close() return true end
    return h
  elseif mode == 'w' or mode == 'wb' then
    local h = {_path = path, _buf = ''}
    function h:write(...)
      for _, v in ipairs({...}) do self._buf = self._buf .. tostring(v) end
      return self
    end
    function h:close() vfs.write(self._path, self._buf); return true end
    return h
  elseif mode == 'a' then
    local h = {_path = path, _buf = ''}
    function h:write(...)
      for _, v in ipairs({...}) do self._buf = self._buf .. tostring(v) end
      return self
    end
    function h:close() vfs.append(self._path, self._buf); return true end
    return h
  end
  return nil, 'unsupported mode: ' .. tostring(mode)
end
\0";
    // SAFETY: SETUP is a valid NUL-terminated Lua chunk; L is non-null.
    let rc = unsafe {
        ffi::luaL_loadbufferx(
            L,
            SETUP.as_ptr() as *const core::ffi::c_char,
            SETUP.len() - 1, // exclude NUL
            c"io_setup".as_ptr(),
            core::ptr::null(),
        )
    };
    if rc == ffi::LUA_OK {
        // SAFETY: L is non-null; pcallk executes the loaded chunk.
        unsafe { ffi::lua_pcallk(L, 0, 0, 0, 0, core::ptr::null_mut()) };
    }
}

#[cfg(not(lua_c_unavailable))]
#[no_mangle]
#[allow(non_snake_case)] // reason: L is the Lua API convention
extern "C" fn main() -> usize {
    // Read spawn args immediately before any heavy initialisation so that the
    // ARGV_STASH_KEY is consumed before the shell overwrites it with the next
    // cell's args.  luaL_newstate + luaL_openlibs are slow; without this early
    // read a second rapid spawn races and both cells receive the later cell's args.
    let mut argbuf_early = [0u8; 512];
    let args_early_len = ostd::syscall::sys_spawn_args(&mut argbuf_early);

    // Install bundled Lua libraries to /tmp so require() finds them.
    install_bundled_scripts();

    // Create the state with Lua's default allocator. It routes through the C
    // malloc family, whose heap is backed by our `__wrap__sbrk` (the glue's
    // static heap) — the toolchain's own `_sbrk` stub returns null.
    // SAFETY: luaL_newstate returns a valid state (or null on OOM).
    let L = unsafe { ffi::luaL_newstate() };
    if L.is_null() {
        ostd::io::println("[lua] out of memory");
        return 1;
    }

    // SAFETY: L is non-null; luaL_openlibs is safe to call once.
    unsafe { ffi::luaL_openlibs(L) };

    // Register the `vfs` table (read/write/append/mkdir/stat/listdir/remove).
    // Net stack delta = 0.
    // SAFETY: L is non-null; binding fns uphold the lua_CFunction contract.
    unsafe {
        ffi::lua_createtable(L, 0, 7);
        ffi::lua_pushcclosure(L, bindings_vfs::vfs_read, 0);
        ffi::lua_setfield(L, -2, c"read".as_ptr());
        ffi::lua_pushcclosure(L, bindings_vfs::vfs_write, 0);
        ffi::lua_setfield(L, -2, c"write".as_ptr());
        ffi::lua_pushcclosure(L, bindings_vfs::vfs_append, 0);
        ffi::lua_setfield(L, -2, c"append".as_ptr());
        ffi::lua_pushcclosure(L, bindings_vfs::vfs_mkdir, 0);
        ffi::lua_setfield(L, -2, c"mkdir".as_ptr());
        ffi::lua_pushcclosure(L, bindings_vfs::vfs_stat, 0);
        ffi::lua_setfield(L, -2, c"stat".as_ptr());
        ffi::lua_pushcclosure(L, bindings_vfs::vfs_listdir, 0);
        ffi::lua_setfield(L, -2, c"listdir".as_ptr());
        ffi::lua_pushcclosure(L, bindings_vfs::vfs_remove, 0);
        ffi::lua_setfield(L, -2, c"remove".as_ptr());
        ffi::lua_setglobal(L, c"vfs".as_ptr());
    }

    // Register ViCell_io_write, then inject io.open/io.write wrappers and
    // strip dangerous stdlib (io.popen, os.execute, debug).
    // SAFETY: L is non-null; binding fns uphold the lua_CFunction contract.
    unsafe {
        ffi::lua_pushcclosure(L, bindings_io::ViCell_io_write, 0);
        ffi::lua_setglobal(L, c"ViCell_io_write".as_ptr());
    }
    // SAFETY: L is non-null; vfs and ViCell_io_write are already registered.
    unsafe { inject_io_setup(L) };

    // Use the args captured before initialisation (avoids the ARGV_STASH_KEY race).
    let args = core::str::from_utf8(&argbuf_early[..args_early_len]).unwrap_or("");

    // `lua -e <code>`: evaluate the chunk and exit (no REPL). The text after
    // "-e " is the Lua source; the shell whitespace-joins argv, so a space-free
    // expression survives intact.
    if let Some(code) = args.strip_prefix("-e ").or_else(|| args.strip_prefix("-e")) {
        let code = code.trim_start();
        // SAFETY: L is valid; eval upholds the Lua-state contract.
        let _ = unsafe { repl_session::eval(L, code) };
        // Park rather than return: the kernel's cell-exit path does not yet
        // unmap a returning cell's address space in the single address space,
        // which corrupts subsequent spawns. Keep the cell alive after
        // evaluating; clean teardown on return is a kernel follow-up.
        loop {
            ostd::task::yield_now();
        }
    }

    // `lua /path/to/script.lua` — read file from VFS and execute.
    // Reached when args is non-empty and does not start with `-e` (the `-e` branch
    // parks before falling through). Empty args falls through to the REPL.
    if !args.is_empty() {
        let path = args.trim();
        let file_buf = vfs_read_to_vec(path);
        if file_buf.is_empty() {
            ostd::io::print("lua: cannot open '");
            ostd::io::print(path);
            ostd::io::println("'");
        } else {
            // Chunk name "@/data/script.lua" for error messages (NUL-terminated).
            let mut chunk_name = alloc::vec![b'@'; 1 + path.len() + 1];
            chunk_name[1..1 + path.len()].copy_from_slice(path.as_bytes());
            *chunk_name.last_mut().unwrap() = 0;
            // SAFETY: L is valid; file_buf contains valid Lua source bytes;
            // chunk_name is NUL-terminated and outlives the pcall.
            let rc = unsafe {
                ffi::luaL_loadbufferx(
                    L,
                    file_buf.as_ptr() as *const core::ffi::c_char,
                    file_buf.len(),
                    chunk_name.as_ptr() as *const core::ffi::c_char,
                    core::ptr::null(),
                )
            };
            if rc == ffi::LUA_OK {
                let _ = unsafe {
                    ffi::lua_pcallk(L, 0, ffi::LUA_MULTRET, 0, 0, core::ptr::null_mut())
                };
            } else {
                let mut len = 0usize;
                // SAFETY: L is valid; -1 is the error string at stack top.
                let ptr = unsafe { ffi::lua_tolstring(L, -1, &mut len as *mut _) };
                if !ptr.is_null() {
                    // SAFETY: Lua guarantees `len` valid bytes at `ptr`.
                    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
                    if let Ok(s) = core::str::from_utf8(bytes) {
                        ostd::io::println(s);
                    }
                }
                // SAFETY: L is valid; pops the error string.
                unsafe { ffi::lua_settop(L, -2) };
            }
        }
        loop { ostd::task::yield_now(); }
    }

    // No `-e`: interactive REPL (multi-line, history, Ctrl+C/D).
    ostd::io::println("Lua 5.4 on ViCell  (Ctrl+D to exit)");
    // SAFETY: L is non-null and valid; run_repl drives the full REPL loop.
    unsafe { repl_session::run_repl(L); }

    // SAFETY: L is non-null; lua_close frees the state.
    unsafe { ffi::lua_close(L) };
    0
}
