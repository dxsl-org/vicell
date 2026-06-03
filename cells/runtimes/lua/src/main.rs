#![no_std]
#![no_main]

extern crate alloc;
extern crate ostd;
extern crate api;

mod bindings_io;
mod bindings_net;
mod bindings_vfs;
mod ffi;
mod repl_session;

/// Read up to 4096 bytes from a VFS path via OP_READ IPC.
///
/// Returns byte count (zero-scan from reply; sys_recv returns sender_id not length).
/// Matches `read_file_vfs` in cells/apps/shell/src/cmd_fs.rs.
fn vfs_read_to_buf(path: &str, buf: &mut [u8]) -> usize {
    const VFS_ENDPOINT: usize = 3;
    const OP_READ: u8 = 8;
    let pb = path.as_bytes();
    let pl = pb.len().min(253) as u8;
    let mut req = [0u8; 256];
    req[0] = OP_READ;
    req[1] = pl;
    req[2..2 + pl as usize].copy_from_slice(&pb[..pl as usize]);
    ostd::syscall::sys_send(VFS_ENDPOINT, &req[..2 + pl as usize]);
    match ostd::syscall::sys_recv(0, buf) {
        ostd::syscall::SyscallResult::Ok(_) => {
            buf.iter().rposition(|&b| b != 0).map(|i| i + 1).unwrap_or(0)
        }
        _ => 0,
    }
}

#[no_mangle]
#[allow(non_snake_case)] // reason: L is the Lua API convention
extern "C" fn main() -> usize {
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

    // Register the `vnet` table (TCP + UDP + DNS).
    // Stack discipline: createtable pushes table @ -1. Each pushcclosure/setfield
    // pair is net-zero. setglobal pops the table. Net delta = 0.
    // SAFETY: L is non-null; binding fns uphold the lua_CFunction contract.
    unsafe {
        ffi::lua_createtable(L, 0, 7);
        ffi::lua_pushcclosure(L, bindings_net::vnet_connect, 0);
        ffi::lua_setfield(L, -2, c"connect".as_ptr());
        ffi::lua_pushcclosure(L, bindings_net::vnet_send, 0);
        ffi::lua_setfield(L, -2, c"send".as_ptr());
        ffi::lua_pushcclosure(L, bindings_net::vnet_recv, 0);
        ffi::lua_setfield(L, -2, c"recv".as_ptr());
        ffi::lua_pushcclosure(L, bindings_net::vnet_close, 0);
        ffi::lua_setfield(L, -2, c"close".as_ptr());
        ffi::lua_pushcclosure(L, bindings_net::vnet_udp_send, 0);
        ffi::lua_setfield(L, -2, c"udp_send".as_ptr());
        ffi::lua_pushcclosure(L, bindings_net::vnet_udp_recv, 0);
        ffi::lua_setfield(L, -2, c"udp_recv".as_ptr());
        ffi::lua_pushcclosure(L, bindings_net::vnet_resolve, 0);
        ffi::lua_setfield(L, -2, c"resolve".as_ptr());
        ffi::lua_setglobal(L, c"vnet".as_ptr());
    }

    // Register the `vfs` table (read/write/append/mkdir). Net stack delta = 0.
    // SAFETY: L is non-null; binding fns uphold the lua_CFunction contract.
    unsafe {
        ffi::lua_createtable(L, 0, 4);
        ffi::lua_pushcclosure(L, bindings_vfs::vfs_read, 0);
        ffi::lua_setfield(L, -2, c"read".as_ptr());
        ffi::lua_pushcclosure(L, bindings_vfs::vfs_write, 0);
        ffi::lua_setfield(L, -2, c"write".as_ptr());
        ffi::lua_pushcclosure(L, bindings_vfs::vfs_append, 0);
        ffi::lua_setfield(L, -2, c"append".as_ptr());
        ffi::lua_pushcclosure(L, bindings_vfs::vfs_mkdir, 0);
        ffi::lua_setfield(L, -2, c"mkdir".as_ptr());
        ffi::lua_setglobal(L, c"vfs".as_ptr());
    }

    // Read the command line published by the spawner (e.g. the shell).
    let mut argbuf = [0u8; 512];
    let n = ostd::syscall::sys_spawn_args(&mut argbuf);
    let args = core::str::from_utf8(&argbuf[..n]).unwrap_or("");

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
        let mut file_buf = alloc::vec![0u8; 4096];
        let n = vfs_read_to_buf(path, &mut file_buf);
        if n == 0 {
            ostd::io::print("lua: cannot open '");
            ostd::io::print(path);
            ostd::io::println("'");
        } else {
            // Chunk name "@/data/script.lua" for error messages (NUL-terminated).
            let mut chunk_name = alloc::vec![b'@'; 1 + path.len() + 1];
            chunk_name[1..1 + path.len()].copy_from_slice(path.as_bytes());
            *chunk_name.last_mut().unwrap() = 0;
            // SAFETY: L is valid; file_buf[..n] is valid Lua source bytes;
            // chunk_name is NUL-terminated and outlives the pcall.
            // luaL_loadbuffer in lua.h is the macro luaL_loadbufferx(L,s,sz,n,NULL);
            // we bind the real symbol and pass null for the mode (text + binary).
            let rc = unsafe {
                ffi::luaL_loadbufferx(
                    L,
                    file_buf.as_ptr() as *const core::ffi::c_char,
                    n,
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
    ostd::io::println("Lua 5.4 on ViOS  (Ctrl+D to exit)");
    // SAFETY: L is non-null and valid; run_repl drives the full REPL loop.
    unsafe { repl_session::run_repl(L); }

    // SAFETY: L is non-null; lua_close frees the state.
    unsafe { ffi::lua_close(L) };
    0
}
