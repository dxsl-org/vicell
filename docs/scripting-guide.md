# Cellos Scripting Guide

Cellos supports two embedded scripting runtimes: **Lua 5.4** (verified) and **MicroPython 1.24.1** (verified).
Both run as native Cells with direct access to the VFS and IPC APIs.

---

## Lua 5.4

### Starting the REPL

At the shell prompt:
```
Cellos> lua
Lua 5.4 on Cellos  (Ctrl+D to exit)
> 
```

Use `Ctrl+D` on an empty line to exit.  `Ctrl+C` cancels the current input line.
Arrow-up/down navigates command history (session-local; persistence added in Phase 17a).

### Running a Script

```
Cellos> exec /bin/lua /scripts/hello.lua
```

(Phase 17a will make `lua script.lua` work directly once arg-passing is wired.)

### Built-in Libraries

All standard Lua 5.4 libraries are available: `string`, `table`, `math`,
`io`, `os`, `coroutine`, `debug`, `package`.

```lua
-- String operations
local s = "Hello, Cellos!"
print(s:upper())          -- HELLO, Cellos!
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
> greet("Cellos")
Hello, Cellos!
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
Cellos> python
MicroPython v1.24.1 on Cellos  (Ctrl+D to exit)
>>> 
```

### Example Script

```python
>>> import os
>>> os.listdir("/bin")
['shell', 'lua', 'python', 'cat', 'ls', ...]
>>> f = open("/readme.txt")
>>> print(f.read())
Welcome to Cellos!
```

### Supported Modules

Standard library modules: `sys`, `os`, `math`, `random`, `json`, `struct`, `hashlib`.
File I/O works via VFS syscalls (read and write).

**Network Module (`vnet`)**:

```python
>>> import vnet
>>> vnet.resolve("gateway")      # Static table: returns "10.0.2.2"
>>> vnet.resolve("google.com")   # DNS query via UDP: returns resolved IP
>>> cap = vnet.socket_tcp()      # Create TCP socket
>>> vnet.connect(cap, "10.0.2.2", 80)
>>> vnet.send(cap, b"GET / HTTP/1.0\r\n\r\n")
>>> vnet.recv(cap, 4096)         # Read up to 4096 bytes
>>> vnet.close(cap)
```

### Running Python Scripts

```
Cellos> exec /bin/python /scripts/hello.py
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

1. Declare the Rust `extern "C" fn Cellos_xxx(L: *mut LuaState) -> c_int` in
   `cells/runtimes/lua/src/bindings_io.rs`.
2. Register it in `cells/runtimes/lua/glue/lua_Cellos_glue.c` via `lua_register(L, "xxx", Cellos_xxx)`.
3. Call any VFS/IPC operations using `ostd::syscall::sys_*` helpers.
4. Add the binding to the table in this document.

---

## Known Limitations (v0.2.1-dev)

| Feature | Status |
|---------|--------|
| **Lua** `io.open` read | ✅ Works (VFS IPC) |
| **Lua** `io.open` write | ✅ Works via vfs.write() IPC |
| **Lua** `os.execute` | Calls `sys_spawn_from_path` (verified) |
| **MicroPython** `open` read | ✅ Works (VFS IPC) |
| **MicroPython** `open` write | ✅ Works via vfs.write() IPC |
| **Both** arg passing to scripts | ✅ Works (spawn_args early-read pattern) |
| **Both** history persistence | 🚧 Phase 17a (VFS write) |
| **Lua** `require` / `package.path` | Stub (no VFS directory scan yet) |

---

## Shell Built-ins

| Built-in | Usage | Added |
|---|---|---|
| echo | Print text, supports $VAR expansion | Phase A |
| cat | Read file (via VFS) | Phase A |
| ls | List directory (via VFS) | Phase A |
| sleep N | Pause N seconds | Phase K |
| source / . | Execute script from VFS | Phase J |
| break / continue | Loop control | Phase R |
| exit N | Exit with code | Phase S |
| unset VAR | Remove variable | Phase S |
| test / [ | Condition testing (-f, -z, -n, =, !=) | Phase U |
| read | Read user input (Phase X-2) | Phase X-2 |
| wget URL path | Download URL body to VFS | Phase U |
| httpd port path | HTTP/1.0 file server | Phase M |

## Shell Advanced Features

| Feature | Usage | Added |
|---|---|---|
| `for var in list; do ... done` | Loop over items | Phase J |
| `while cond; do ... done` | While loop | Phase R |
| `if cond; then ... fi` | Conditional | Phase S |
| `case var in pattern) ... esac` | Case switch | Phase T |
| `name() { ... }` | Define shell function | Phase X-1 |
| `func $1 $2 $9` | Function arguments | Phase X-2 |
| `$(cmd)` | Command substitution | Phase X-3 |
| `\| grep` | Pipe commands | Phase E |
| `> file` | Redirect stdout to file | Phase E |
| `>> file` | Append stdout to file | Phase E |
| `& bg` | Background job | Phase F |
| `$VAR` | Variable expansion | Phase G |
| `$?` | Last exit code | Phase H |
| `||` and `&&` | Command chaining | Phase I |
