# Tier 1b Lua — Dynamic Scripting

> Lua interpreter Cell for quick scripting, CLI tools, and dynamic workloads.

---

## Overview

Cellos includes a **Lua interpreter Cell** at `cells/runtimes/lua/`. Scripts run in a sandbox with:

- **VFS bindings**: read/write files via IPC
- **Restricted stdlib**: `os`, `io.popen`, `debug` disabled (no network syscalls)
- **Performance**: ~50× slower than Tier 1 Rust (acceptable for scripts/tools)

---

## Running a Script

From the shell:

```bash
lua /path/to/script.lua
```

Or interactively:

```bash
lua   # starts REPL
> print("hello")
hello
> os.exit()
```

---

## Lua Standard Library (What's Available)

| Module | Status | Notes |
|--------|--------|-------|
| `io` | ⚠️ Restricted | `io.open`, `io.write`, `io.read` only; `io.popen` disabled |
| `os` | ⚠️ Restricted | `os.date`, `os.time` only; `os.execute` / `os.remove` disabled |
| `string`, `table`, `math` | ✅ Full | No restrictions |
| `coroutine` | ✅ Full | Coroutines work |
| `utf8` | ✅ Full | UTF-8 string operations |
| `debug` | ❌ Disabled | Security boundary — stripped at init |

---

## VFS Bindings

Read and write files via Cellos's VFS service:

```lua
-- Read entire file
local content = vfs.read("/bin/shell")
if content then
    print("Read " .. #content .. " bytes")
else
    print("File not found")
end

-- Write entire file (creates or truncates)
vfs.write("/tmp/output.txt", "hello world\n")

-- Append to file
vfs.append("/tmp/output.txt", "more data\n")

-- Check if file exists (stat)
local stat = vfs.stat("/tmp")
if stat then
    print("Size=" .. stat.size .. " is_dir=" .. (stat.is_dir and "yes" or "no"))
else
    print("File not found")
end

-- List directory
local entries = vfs.listdir("/")
if entries then
    for i, name in ipairs(entries) do
        print(i .. ": " .. name)
    end
end

-- Delete (remove)
vfs.remove("/tmp/old.txt")

-- Create directory
vfs.mkdir("/tmp/newdir")
```

All VFS operations return `nil` on error (no exceptions). Always check return values.

---

## Network (Not Available)

The Lua Cell has **no network socket syscalls**. To send/receive data:

1. Write to VFS (files)
2. Let another Cell read and forward via network
3. Or use a named pipe / socket file (future)

This is the **no-network constraint** — it protects against malicious scripts.

---

## Example: JSON Parser

```lua
local json = require("json")

local data = '{"name":"Cellos","version":"0.3.0"}'
local parsed = json.parse(data)
print(parsed.name)        -- Cellos
print(parsed.version)     -- 0.3.0

local output = json.stringify({ status = "ok", code = 200 })
vfs.write_file("/tmp/response.json", output)
```

The bundled `json.lua` module (MIT license) is pre-installed at startup; `require("json")` works out of the box.

---

## Example: CLI Tool

```lua
#!/bin/lua

local args = {...}  -- command-line arguments

if #args == 0 then
    print("Usage: mytool <file>")
    os.exit(1)
end

local file = args[1]
local content = vfs.read_file(file)
if not content then
    print("Error: cannot read " .. file)
    os.exit(1)
end

-- Process content
local lines = {}
for line in content:gmatch("[^\n]+") do
    table.insert(lines, string.upper(line))
end

-- Write output
vfs.write_file("/tmp/mytool.out", table.concat(lines, "\n"))
print("Done: wrote /tmp/mytool.out")
```

---

## Coroutines

Lua coroutines are fully supported (no restriction):

```lua
local function slow_generator()
    for i = 1, 5 do
        coroutine.yield(i * 10)
    end
end

local co = coroutine.create(slow_generator)
while true do
    local ok, val = coroutine.resume(co)
    if not ok or val == nil then break end
    print(val)  -- 10, 20, 30, 40, 50
end
```

---

## Performance & Limits

- **Speed**: ~50× slower than Tier 1 Rust (acceptable for 100-ms+ tasks)
- **Heap**: shared with the Lua runtime; no per-script limit
- **Timeout**: runs until `os.exit()` or kernel watchdog (see `sys_heartbeat`)

For performance-critical work, drop to Tier 1 Rust or Tier 1b C.

---

## Manifest & Syscalls

Built-in; no customization needed. The Lua Cell declares:

```rust
api::declare_manifest!(block_io = false, network = false, spawn = false);
api::declare_syscalls![Send, Recv, Log, Heartbeat];
```

Scripts access VFS via IPC (the Lua Cell forwards requests).

---

## Bundled Libraries

Installed to `/tmp` at startup:

- `json.lua` — JSON encode/decode (MIT)
- `json_test.lua` — Test suite for json.lua
- `coroutine_test.lua` — Coroutine demo

Access via `require("json")` or `dofile("/tmp/json.lua")`.

---

## When to Use Tier 1b Lua

✅ Quick prototyping, one-off scripts  
✅ CLI tools, text processing  
✅ Configuration / task automation  
✅ Learning Cellos syscalls (no Rust needed)  

❌ Performance-critical code → use Tier 1 Rust  
❌ Network apps → use Tier 1 Rust + SDK L1  
❌ Untrusted code → use Tier 3b (Linux VM, G2+)  

---

## Canonical Example

See [cells/runtimes/lua/src/main.rs](../../cells/runtimes/lua/src/main.rs) — the interpreter Cell entry point. Scripts run via REPL or file (`lua /path`).

---

## Troubleshooting

**Script hangs / doesn't exit?**  
→ Lua is waiting for input (REPL mode). Use `os.exit()` or press Ctrl+C (if supported).

**VFS returns nil?**  
→ Service not registered or file doesn't exist. Check kernel boot output.

**io.popen / os.execute fails?**  
→ These are intentionally disabled (security boundary). Use VFS + separate Cells instead.

---

## Next Steps

- Need performance? → [Tier 1 Rust](tier1-rust-sdk.md)
- Have C/C++ code? → [Tier 1b C/Zig](tier1b-c-zig.md)
- See [scripting-guide.md](../scripting-guide.md) for Lua ecosystem integration.
