# ViOS Scripting Guide

ViOS supports two embedded scripting runtimes: **Lua 5.4** (verified) and **MicroPython 1.24.1** (verified).
Both run as native Cells with direct access to the VFS and IPC APIs.

---

## Lua 5.4

### Starting the REPL

At the shell prompt:
```
ViOS> lua
Lua 5.4 on ViOS  (Ctrl+D to exit)
> 
```

Use `Ctrl+D` on an empty line to exit.  `Ctrl+C` cancels the current input line.
Arrow-up/down navigates command history (session-local; persistence added in Phase 17a).

### Running a Script

```
ViOS> exec /bin/lua /scripts/hello.lua
```

(Phase 17a will make `lua script.lua` work directly once arg-passing is wired.)

### Built-in Libraries

All standard Lua 5.4 libraries are available: `string`, `table`, `math`,
`io`, `os`, `coroutine`, `debug`, `package`.

```lua
-- String operations
local s = "Hello, ViOS!"
print(s:upper())          -- HELLO, VIOS!
print(#s)                 -- 12

-- Math
print(math.sqrt(144))     -- 12.0
print(math.floor(3.7))    -- 3

-- Table
local t = {1, 2, 3, "four"}
for i, v in ipairs(t) do
    print(i, v)
end
```

### VFS I/O

The `io.open` binding wraps the VFS service IPC.  Read-only access works in v1.0.

```lua
local f = io.open("/readme.txt", "r")
if f then
    local content = io.read(f, 4096)
    io.close(f)
    print(content)
else
    print("file not found")
end
```

Write access requires the FAT32 VFS integration (Phase 13 FAT32 milestone).

### os.execute

Spawn a shell command (stub in v1.0 — prints the command, returns 0):

```lua
local rc = os.execute("ls /bin")
print("exit:", rc)
```

Full shell spawning via `SpawnFromPath` is wired in Phase 17a.

### Multi-line Input

The Lua REPL handles incomplete chunks automatically — just keep typing:

```
> function greet(name)
>>   print("Hello, " .. name .. "!")
>> end
> greet("ViOS")
Hello, ViOS!
```

### Example Scripts

#### Fibonacci
```lua
local function fib(n)
    if n <= 1 then return n end
    return fib(n-1) + fib(n-2)
end
for i = 0, 10 do
    io.write(fib(i) .. " ")
end
print()
-- 0 1 1 2 3 5 8 13 21 34 55
```

#### Read and parse a config file
```lua
local f = io.open("/etc/hostname", "r")
if f then
    local name = io.read(f, 256)
    io.close(f)
    print("Hostname:", name)
end
```

---

## MicroPython 1.24.1 (Verified)

MicroPython is available as `/bin/python` with an interactive REPL and script runner.
The runtime includes a 256 KB heap and FFI bindings to the VFS and IPC APIs.

### Starting the REPL

```
ViOS> python
MicroPython v1.24.1 on ViOS  (Ctrl+D to exit)
>>> 
```

### Example Script

```python
>>> import os
>>> os.listdir("/bin")
['shell', 'lua', 'python', 'cat', 'ls', ...]
>>> f = open("/readme.txt")
>>> print(f.read())
Welcome to ViOS!
```

### Supported Modules

Standard library modules: `sys`, `os`, `math`, `random`, `json`, `struct`, `hashlib`.
File I/O works via VFS syscalls (read-only for now).

### Running Python Scripts

```
ViOS> exec /bin/python /scripts/hello.py
```

---

## Examples Directory

Place `.lua` and `.py` scripts under `/scripts/` on the disk image.
Use `scripts/format-disk.ps1` to bake them in.

```
disk.img
└── scripts/
    ├── hello.lua
    ├── fib.lua
    └── hello.py  (planned)
```

---

## Adding New Lua C Bindings

1. Declare the Rust `extern "C" fn vios_xxx(L: *mut LuaState) -> c_int` in
   `cells/runtimes/lua/src/bindings_io.rs`.
2. Register it in `cells/runtimes/lua/glue/lua_vios_glue.c` via `lua_register(L, "xxx", vios_xxx)`.
3. Call any VFS/IPC operations using `ostd::syscall::sys_*` helpers.
4. Add the binding to the table in this document.

---

## Known Limitations (v0.2.1-dev)

| Feature | Status |
|---------|--------|
| **Lua** `io.open` read | ✅ Works (VFS IPC) |
| **Lua** `io.open` write | 🚧 Phase 13 FAT32 |
| **Lua** `os.execute` | Calls `sys_spawn_from_path` (verified) |
| **MicroPython** `open` read | ✅ Works (VFS IPC) |
| **MicroPython** `open` write | 🚧 Phase 13 FAT32 |
| **Both** arg passing to scripts | 🚧 Phase 17a |
| **Both** history persistence | 🚧 Phase 17a (VFS write) |
| **Lua** `require` / `package.path` | Stub (no VFS directory scan yet) |
