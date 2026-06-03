#![no_std]
#![no_main]

extern crate ostd;
extern crate api;

mod bindings_io;
mod bindings_net;
mod ffi;
mod repl_session;

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

    // No `-e`: interactive REPL (multi-line, history, Ctrl+C/D).
    ostd::io::println("Lua 5.4 on ViOS  (Ctrl+D to exit)");
    // SAFETY: L is non-null and valid; run_repl drives the full REPL loop.
    unsafe { repl_session::run_repl(L); }

    // SAFETY: L is non-null; lua_close frees the state.
    unsafe { ffi::lua_close(L) };
    0
}
