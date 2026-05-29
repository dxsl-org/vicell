# Phase 10 — Lua C Binding via `cc` Crate

**Effort:** 40h | **Priority:** P1 | **Status:** complete | **Blockers:** none

## Overview

The `cells/runtimes/lua` cell currently stubs Lua FFI. Link the real Lua 5.4 C source (vendored) into the cell via the `cc` crate, exposing a usable Lua interpreter that runs `.lua` scripts on ViOS. Provides validation of OSTD's libc shim and unlocks Phase 18 (full runtime enhancement).

## Context Links

- `docs/05-application.md` — application model: native, WASM, VM/scripted
- `libs/api/src/posix.rs` — POSIX shim recently merged (Phase 01 verified)
- `libs/api/src/c/` — C type definitions for FFI
- Lua reference: 5.4.x release tarball from lua.org (MIT license, compatible with our license stance)

## Key Insights

- Lua 5.4 is ANSI C; with `LUA_USE_POSIX` undefined, requires only `<stddef.h>`, `<stdint.h>`, `<string.h>`, `<setjmp.h>`, a few stdio routines, and malloc/free. Most of these the POSIX shim already provides; the missing pieces (mostly `<setjmp.h>` and `printf` family) we provide via OSTD.
- The `cc` crate compiles C with the cross-toolchain dictated by `CC_<target>` env var. Cargo's default for `riscv64gc-unknown-none-elf` typically resolves to `riscv64-unknown-elf-gcc` (must be in PATH) or `clang --target=riscv64-unknown-none-elf`.
- Pin a specific Lua version (5.4.7 as of 2026-05) and **vendor** the C source under `cells/runtimes/lua/vendor/lua-5.4.7/`. Do not depend on internet at build time.
- `<setjmp.h>` is a portability minefield in freestanding contexts. Use Lua's `LUAI_THROW`/`LUAI_TRY` overrides to switch to a C++-style mechanism — OR provide a minimal `setjmp`/`longjmp` per architecture (RV64 via 32-reg save).
- Patching: Lua expects `LUAI_THROW`, `lua_writestring`, `lua_writeline`, `lua_writestringerror` to be defined. Provide via a small `lua_vios_glue.c` rather than patching pristine Lua sources.

## Requirements

**Functional**
- `cargo build -p lua --release` succeeds for RV64 + AArch64 + x86_64
- Lua cell `lua` binary runs in shell: `lua -e "print(2+2)"` outputs `4`
- `lua script.lua` reads file via VFS (Phase 13 deeper integration) — for this phase, accept stdin or compiled-in script
- All Lua sources unmodified (no in-place edits); customization via glue C file

**Non-functional**
- Build time of lua cell < 60s on cold cache
- Output binary < 1.5 MB
- Zero `unsafe` in lua-cell Rust code outside the FFI boundary

## Architecture

```
cells/runtimes/lua/
├── vendor/lua-5.4.7/      ← unmodified upstream
│   ├── src/lapi.c, lauxlib.c, lbaselib.c, …
│   └── README
├── glue/
│   ├── lua_vios_glue.c    ← LUAI_THROW, lua_writestring etc.
│   ├── lua_vios_setjmp.S  ← per-arch setjmp/longjmp asm
│   └── lua_vios_config.h  ← LUA_USE_C89 etc.
├── build.rs               ← cc-crate compile + glob
├── src/main.rs            ← Rust entry, FFI calls into Lua
└── Cargo.toml
```

`build.rs` compiles every C file in `vendor/lua-5.4.7/src/*.c` (except `lua.c` and `luac.c` which define their own main) + the glue files, into a static lib. Rust `extern "C"` bindings call `luaL_newstate`, `luaL_dostring`, etc.

## Related Code Files

**Modify:**
- `cells/runtimes/lua/Cargo.toml` — add `[build-dependencies] cc = "1.0"`, optionally `[dependencies] cty` for C types
- `cells/runtimes/lua/build.rs` — invoke cc-crate
- `cells/runtimes/lua/src/main.rs` — Rust-side: parse args, create Lua state, dostring/dofile, print errors

**Create:**
- `cells/runtimes/lua/vendor/lua-5.4.7/` — full upstream extracted (added to repo)
- `cells/runtimes/lua/vendor/lua-5.4.7/LICENSE` — vendored MIT license copy
- `cells/runtimes/lua/glue/lua_vios_glue.c` — overrides for missing libc bits + Lua hooks
- `cells/runtimes/lua/glue/lua_vios_setjmp.S` — minimal setjmp/longjmp for each arch (RV64 + AArch64 + x86_64)
- `cells/runtimes/lua/glue/lua_vios_config.h` — Lua config header defining LUA_USE_C89, custom LUAI_THROW
- `cells/runtimes/lua/src/ffi.rs` — Rust FFI bindings for `lua_State`, `luaL_newstate`, `luaL_openlibs`, `luaL_loadstring`, `lua_pcall`, `lua_tostring`, etc.
- `cells/runtimes/lua/tests/smoke.lua` — `print("ok")` script, used by integration test
- `tests/integration/lua_smoke.rs` — boot, exec `lua -e "print(2+2)"`, assert `4`

## Implementation Steps

1. **Download + vendor Lua 5.4.7**:
   - Fetch from https://www.lua.org/ftp/lua-5.4.7.tar.gz
   - Verify SHA256 against published value (record in vendor README)
   - Extract into `cells/runtimes/lua/vendor/lua-5.4.7/`; do not modify any file
2. **Create `glue/lua_vios_config.h`**:
   ```c
   #pragma once
   #define LUA_USE_C89
   #define LUAI_THROW(L,c)   longjmp((c)->b, 1)
   #define LUAI_TRY(L,c,a)   if (setjmp((c)->b) == 0) { a }
   #define luai_jmpbuf       jmp_buf
   /* Provide our own write helpers in glue */
   void vios_write(const char* s, size_t n);
   #define lua_writestring(s,l)    vios_write((s), (l))
   #define lua_writeline()         vios_write("\n", 1)
   #define lua_writestringerror(s,p)  vios_write((s), strlen(s))
   ```
3. **Create `glue/lua_vios_glue.c`**:
   - Define `vios_write` calling `ostd::console::write_bytes` via an `extern "Rust"` thunk
   - Provide any `<stdio.h>` shims Lua libs need (fprintf to stderr, etc.)
   - Provide `abort()` calling `ostd::task::exit(1)`
4. **Create per-arch setjmp/longjmp** (`glue/lua_vios_setjmp.S`):
   - RV64: save x1, x2, x8, x9, x18..x27, f8, f9, f18..f27
   - AArch64: save x19..x29, sp, lr, d8..d15
   - x86_64: save rbx, rbp, r12..r15, rsp, return address
   - Each arch behind `#if defined(__riscv) / __aarch64__ / __x86_64__`
5. **Write `build.rs`**:
   ```rust
   fn main() {
       let mut b = cc::Build::new();
       b.warnings(false)
        .flag("-std=c89")
        .define("LUA_USE_C89", None)
        .include("vendor/lua-5.4.7/src")
        .include("glue");
       for entry in std::fs::read_dir("vendor/lua-5.4.7/src").unwrap() {
           let p = entry.unwrap().path();
           if let Some("c") = p.extension().and_then(|x| x.to_str()) {
               let name = p.file_stem().unwrap().to_str().unwrap();
               if matches!(name, "lua" | "luac") { continue; }
               b.file(&p);
           }
       }
       b.file("glue/lua_vios_glue.c");
       b.file("glue/lua_vios_setjmp.S");
       b.compile("lua54");
   }
   ```
6. **Rust FFI in `src/ffi.rs`**:
   ```rust
   #[repr(C)] pub struct LuaState { _private: [u8;0] }
   extern "C" {
       pub fn luaL_newstate() -> *mut LuaState;
       pub fn luaL_openlibs(L: *mut LuaState);
       pub fn luaL_loadstring(L: *mut LuaState, s: *const u8) -> i32;
       pub fn lua_pcall(L: *mut LuaState, nargs: i32, nresults: i32, errfunc: i32) -> i32;
       pub fn lua_tolstring(L: *mut LuaState, idx: i32, len: *mut usize) -> *const u8;
       pub fn lua_close(L: *mut LuaState);
   }
   ```
7. **Rust `main.rs`**:
   - Parse args: support `-e <code>` (eval), `<file>` (open via VFS, dofile)
   - Create state, openlibs, loadstring + pcall
   - On error: print Lua error, exit nonzero
   - Drop state via lua_close (RAII wrapper)
8. **Smoke test**:
   - `cells/runtimes/lua/tests/smoke.lua` contains `print(2+2)`
   - `tests/integration/lua_smoke.rs` boots ViOS, drives shell to run lua, captures `4`
9. **Wire Lua into shell PATH** by adding `/bin/lua` to disk image (gen_disk.ps1)
10. **Document**:
   - `cells/runtimes/lua/README.md` — version pin, glue overview, how to bump Lua
   - License notice: Lua MIT (already permissive) — incorporate in NOTICE if we have one

## Todo List

- [ ] Vendor lua-5.4.7 + record SHA256 in README
- [ ] Create `lua_vios_config.h`
- [ ] Create `lua_vios_glue.c` (write/abort/stdio shims)
- [ ] Create per-arch `lua_vios_setjmp.S` (RV64, AArch64, x86_64)
- [ ] Write `build.rs` with cc-crate glob
- [ ] Implement Rust FFI in `src/ffi.rs`
- [ ] Implement `main.rs` (arg parsing, run loop)
- [ ] Create `tests/smoke.lua` + integration test
- [ ] Add `/bin/lua` to disk image
- [ ] Document version + bump procedure in `cells/runtimes/lua/README.md`
- [ ] Lua cell builds for RV64 + AArch64 + x86_64 in CI

## Success Criteria

- `lua -e "print(2+2)"` outputs `4` in shell
- `lua /scripts/smoke.lua` (where smoke.lua is in disk image) succeeds
- Build time < 60s cold, < 5s incremental
- Binary size < 1.5 MB
- Lua cell compiles for all 3 primary archs

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| `<setjmp.h>` semantics differ from real Lua expectations | High | High | Use longjmp asm carefully; add a self-test (raise/catch loop) before exposing to scripts |
| Lua's GC interacts badly with no_std allocator | Med | Med | Set `LUAI_MAXCSTACK` low (200); enable explicit `collectgarbage()` calls in long scripts |
| C cross-compiler not in CI env | Med | Low | Install `gcc-riscv64-linux-gnu` and equivalents in `.github/workflows/ci.yml` |
| Vendored sources balloon repo size (+~500KB) | Cert | Low | Acceptable; alternative is a submodule (more friction) |
| Future Lua releases break glue config | Low | Low | Pin version; bump via dedicated PR with full test pass |

## Security Considerations

- Lua scripts run in the cell's address space — they have the cell's capabilities. Scripts are fully trusted within their cell.
- Limit C stack: configure `LUAI_MAXCCALLS` to bound recursion (prevent stack-blow DoS by malicious scripts in a cell).
- File I/O from Lua goes through OSTD → VFS IPC → respects capability model.
- No `os.execute`, no `loadfile` with arbitrary paths in v1.0; sandbox by selectively `openlibs` (skip `os` lib if running untrusted scripts in a public Cell).

## Rollback

Revert removes vendor/, glue/, and reverts main.rs to stub. Lua cell becomes the previous stub. No external dependencies; rollback is purely additive to remove.

## Next Steps

Phase 18 expands Lua with full stdlib access, VFS I/O integration, and an interactive REPL. MicroPython gets a similar treatment in Phase 18. Pattern (vendored C + cc crate + glue) is reusable for any future C library port.
