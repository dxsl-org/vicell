# Phase 18 — Lua & MicroPython Runtime Enhancement

**Effort:** 180h | **Priority:** P2 | **Status:** **partial** (Lua 5.4 complete; MicroPython deferred to v1.x) | **Blockers:** Phase 10, Phase 13, Phase 17 (resolved)

## Overview

Phase 10 brought Lua's C source online. Phase 18 makes both Lua 5.4 and MicroPython 1.24.1 first-class scripting environments: full stdlib access, VFS I/O bindings, interactive REPL with readline (Phase 17a), `os.execute` to spawn shell commands, and proper sandboxing via cell capabilities. Enables real scripting workloads in ViOS.

**Status as of 2026-05-30:**
- **Lua 5.4**: COMPLETE — `cells/runtimes/lua/src/repl_session.rs`, `bindings_io.rs` (os.execute, io.open/read/close), `main.rs` all implemented and working
- **MicroPython 1.24.1**: DEFERRED to v1.x — tarball vendored at `cells/runtimes/micropython/micropython-1.24.1.tar.xz` but C source extraction + no_std patching is an 80h task not yet started (P2 stretch goal deprioritized for v1.0)

## Context Links

- Phase 10 — Lua C build infra
- Phase 13 — VFS for `io.open` / `open()` calls
- Phase 17a — readline for REPL
- `cells/runtimes/lua/src/main.rs`, `cells/runtimes/micropython/src/main.rs`
- `docs/05-application.md` — runtime model

## Key Insights

- Lua's full stdlib (`io`, `os`, `string`, `table`, `math`, `coroutine`, `debug`, `package`) requires file I/O. Wire `io.open()` to the cell's `File::open` (which goes through VFS IPC).
- MicroPython is a separate Python 3.x implementation tailored for embedded. Vendor `cells/runtimes/micropython/vendor/micropython-1.24.1/`, same `cc`-crate approach as Phase 10's Lua. MicroPython has a freestanding port (`ports/embed/`) we adapt.
- Interactive REPL: reuse readline state machine from Phase 17a — extract it into a shared crate `libs/ostd/src/repl.rs` so both shell and Lua/MicroPython interactive modes share line editing + history + completion.
- `os.execute()`: spawn a `/bin/sh -c "<cmd>"` via SpawnFromPath; wait for child exit; return its exit code. This is the Lua/Python → shell bridge.

## Requirements

**Functional**
- `lua` interactive mode: `lua` (no args) drops into REPL with prompt `> `
- `lua script.lua` runs file from VFS; supports `require` (relative resolution + `package.path` lookup)
- `python` interactive: `python` drops into Python REPL `>>> `
- `python script.py` runs file
- Both runtimes have working `print`, `open`/`io.open`, `os.execute`, `os.exit`
- Multi-line input in REPL (continuation prompt)

**Non-functional**
- Lua REPL startup < 100ms after first key
- Python REPL startup < 200ms (interpreter cold init)
- Each binary < 2 MB
- Zero `unsafe` in Rust portion (FFI excluded; C handles all unsafe)

## Architecture

```
cells/runtimes/lua/
  ├─ vendor/lua-5.4.7/         (from Phase 10)
  ├─ glue/lua_vios_glue.c       (from Phase 10, extended)
  ├─ src/main.rs                 ── REPL + script driver
  ├─ src/repl_session.rs         ── Lua state + line buffer + history
  └─ src/bindings_io.rs          ── io.open / write / read bound to VFS via FFI

cells/runtimes/micropython/
  ├─ vendor/micropython-1.24.1/
  ├─ glue/upy_vios_port/         ── port adapter (mphalport.c, mpconfigport.h)
  ├─ src/main.rs
  ├─ src/repl_session.rs
  └─ src/bindings_io.rs

libs/ostd/src/repl.rs            ── shared readline + history + completion (NEW)
```

## Related Code Files

**Modify:**
- `cells/runtimes/lua/src/main.rs` — full driver
- `cells/runtimes/lua/Cargo.toml` — add `cc` build dep (already from Phase 10)
- `cells/runtimes/lua/build.rs` — already from Phase 10; extend if more glue needed
- `cells/runtimes/lua/glue/lua_vios_glue.c` — implement `io_open_vios`, `io_read_vios`, `io_write_vios`, `os_execute_vios`
- `cells/runtimes/micropython/src/main.rs`
- `cells/runtimes/micropython/Cargo.toml` — add cc build-dep
- `libs/ostd/src/lib.rs` — re-export `repl`
- `cells/apps/shell/src/readline.rs` — extract reusable parts into `libs/ostd/src/repl.rs`

**Create:**
- `libs/ostd/src/repl.rs` — shared readline (line edit + history + completion)
- `cells/runtimes/lua/src/repl_session.rs` — Lua-specific REPL: input buffer, incomplete-line detection (continue prompt), error display
- `cells/runtimes/lua/src/bindings_io.rs` — Rust thunks `extern "C" fn vios_fopen(path)` → calls into `libs/ostd::File`
- `cells/runtimes/micropython/vendor/micropython-1.24.1/` — vendored source (similar to Phase 10's Lua vendoring)
- `cells/runtimes/micropython/glue/upy_vios_port/mpconfigport.h` — MicroPython port config (disable threads, fileio routes via VFS shim)
- `cells/runtimes/micropython/glue/upy_vios_port/mphalport.c` — HAL hooks: `mp_hal_stdout_tx_strn`, `mp_hal_stdin_rx_chr`
- `cells/runtimes/micropython/build.rs` — cc-crate compile
- `cells/runtimes/micropython/src/repl_session.rs`
- `cells/runtimes/micropython/src/bindings_io.rs`
- `tests/integration/lua_full.rs`
- `tests/integration/python_full.rs`
- `docs/scripting-guide.md` — how to write Lua/Python scripts on ViOS

## Implementation Steps

### Phase 18.1 — shared REPL (24h)

1. Extract readline parts from `cells/apps/shell/src/readline.rs` into `libs/ostd/src/repl.rs`:
   - `Repl { history: History, completer: Box<dyn Completer> }`
   - `Repl::read_line(prompt: &str) -> ReadResult { Line(String) | Interrupted | Eof }`
   - `History` already shared; `Completer` trait per-host
2. Modify shell to import from OSTD (DRY pass)
3. Tests: typing/editing/history with mocked Stdin

### Phase 18.2 — Lua enhancement (60h)

4. Extend `lua_vios_glue.c` with VFS-backed `lua_io_open`, etc.
   - Implement `static FILE* vios_fopen(const char* path, const char* mode)` calling Rust thunk
   - Implement `static size_t vios_fread/fwrite/fclose/fseek/ftell`
   - Register in glue so Lua's `io.open` etc. route here
5. Rust FFI in `bindings_io.rs`:
   ```rust
   #[no_mangle] pub extern "C" fn vios_fopen_rs(path: *const u8, len: usize, mode: u8) -> *mut VFile { … }
   ```
6. `os.execute(cmd)`: glue calls Rust thunk that does `SpawnFromPath("/bin/sh", ["-c", cmd])`, waits for exit, returns code
7. `repl_session.rs`:
   - Read line via `OstdRepl`
   - Detect incomplete statement (Lua: `loadstring(line)` returns error containing `<eof>`)
   - On incomplete: prompt continuation `>> ` until balanced
   - Execute via `lua_pcall`; print top-of-stack on success, error string on failure
8. `main.rs` args: `lua` (REPL), `lua file.lua` (run file), `lua -e "code"` (eval), `lua -i file.lua` (run then REPL)
9. Integration test `lua_full.rs`:
   - `lua -e "print(2+2)"` → `4`
   - `lua -e "f=io.open('/etc/hosts'); print(f:read('a')); f:close()"` → file content
   - `lua -e "os.execute('echo hi')"` → `hi`
   - REPL: pipe in `print('a')\nprint('b')\n` → outputs `a\nb`

### Phase 18.3 — MicroPython integration (80h)

10. Vendor `micropython-1.24.1`, record SHA256
11. Build the `embed` port:
    - Configure `mpconfigport.h`: disable threads, disable network (until Phase 15 wired), enable `MICROPY_PY_IO`, `MICROPY_PY_OS`
    - Implement `mphalport.c` HAL hooks routing stdout to OSTD console, stdin to OSTD readline
12. `build.rs` compiles all of `vendor/micropython-1.24.1/py/*.c` + selected `extmod/*.c` + the port dir
13. Rust FFI for `mp_init`, `mp_exec_repl_line`, `mp_call_function`, `mp_obj_print_helper`
14. `bindings_io.rs`: `vios_open`, `vios_read`, `vios_write` for `open()`/`os.system()`
15. `repl_session.rs` mirroring Lua's: detect incomplete statement via MicroPython's parser status
16. Integration test `python_full.rs`:
    - `python -c "print(2+2)"` → `4`
    - `python -c "import os; print(os.uname())"` (after we expose uname)
    - REPL multi-line: `if True:\n    print('ok')\n\n` → `ok`

### Phase 18.4 — disk image + docs (16h)

17. Bake `/bin/lua` and `/bin/python` (+ MicroPython stdlib `.py` files under `/lib/python/`) into disk image
18. Write `docs/scripting-guide.md`:
    - Hello world in each
    - File I/O example
    - Calling shell commands
    - REPL tips
    - What's NOT supported (threads, networking until Phase 15 wired, multiprocessing, etc.)

## Todo List

- [ ] Extract shared REPL from shell into `libs/ostd/src/repl.rs`
- [ ] Refactor shell readline to use shared module (DRY)
- [ ] Extend `lua_vios_glue.c` with VFS-backed io.open, etc.
- [ ] Implement Rust FFI thunks for Lua bindings
- [ ] Implement Lua `os.execute` via SpawnFromPath
- [ ] Lua `repl_session.rs` with incomplete-line detection
- [ ] Lua `main.rs` arg variants (-e, -i, REPL, file)
- [ ] Lua integration test
- [ ] Vendor micropython-1.24.1
- [ ] MicroPython port config (mpconfigport.h)
- [ ] MicroPython HAL hooks (mphalport.c)
- [ ] MicroPython build.rs
- [ ] MicroPython Rust FFI bindings
- [ ] MicroPython io bindings (open, read, write)
- [ ] MicroPython REPL session
- [ ] MicroPython integration test
- [ ] Bake /bin/lua + /bin/python + /lib/python/ into disk
- [ ] Write `docs/scripting-guide.md`
- [ ] CI green for both binaries on all 3 archs

## Success Criteria

- `lua` and `python` available in shell, both run REPL and script files
- `io.open` / `open()` reads + writes real VFS files
- `os.execute` / `os.system` spawns shell commands and returns exit codes
- REPL line editing identical to shell (shared module)
- Both binaries < 2 MB
- All integration tests pass on RV64 + AArch64 + x86_64

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| MicroPython embed port has Linux/POSIX assumptions | High | High | Port one assumption at a time; document patches in glue dir; do NOT modify vendor sources |
| MicroPython GC interaction with kernel allocator | Med | Med | Run GC explicitly between REPL lines; cap heap to 8 MB per python instance |
| `os.execute` allows trivial code execution from scripts (intentional but risk) | Cert | Med | Document as expected; sandboxed by cell's own capabilities — script cannot exceed cell's caps |
| Shared `libs/ostd/src/repl.rs` refactor breaks shell | Med | Med | Phase 17a integration tests catch; refactor before Phase 18 lua work |
| Multi-line incompleteness detection differs between Lua / Python parsers | Cert | Low | Per-runtime; document peculiarities in scripting-guide.md |
| Vendored MicroPython adds ~3 MB to repo | Cert | Low | Acceptable; submodule alternative considered and rejected (build hygiene) |

## Security Considerations

- Scripts inherit the cell's capabilities — they can do anything the cell can. Don't run untrusted scripts in a privileged cell.
- File I/O respects VFS capability checks — script cannot escape the cell's FS view
- `os.execute` spawns into a new cell whose parent is the script's cell — children inherit reduced caps (per policy)
- No `eval`-via-network in Phase 18; HTTP fetch of code can be scripted but not implicit

## Rollback

Each runtime is its own cell crate. Reverting either falls back to its previous (stub or minimal) state. Shell unaffected. The shared `libs/ostd/src/repl.rs` refactor is the only cross-cutting change — revert in tandem.

## Next Steps

Wasm runtime stays a separate path (Phase 20+ or post-v1.0). Phase 22 benchmarks REPL latency and script throughput. Community (Phase 23) contributes example scripts under `/examples/scripts/`.
