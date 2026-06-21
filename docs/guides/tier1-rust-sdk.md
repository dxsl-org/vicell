# Tier 1 Rust + SDK L1 — Apps with Services

> VFS, network, and IPC clients built into AppContext. For most user apps.

---

## AppContext: The Entry Point

Instead of raw syscalls, use the context:

```rust
#![no_std]
#![no_main]

extern crate alloc;

use ostd::app::{AppContext, AppEvent};
use ostd::io::println;

ostd::app_entry!(handler = app_main);

fn app_main(ctx: &mut AppContext, event: AppEvent) {
    match event {
        AppEvent::Init => {
            // ctx.vfs(), ctx.net(), ctx.send() all available here
            match ctx.vfs().stat("/") {
                Ok((size, is_dir)) => {
                    println(&alloc::format!("Root: size={} is_dir={}", size, is_dir));
                }
                Err(_) => println("VFS unavailable"),
            }
        }
        AppEvent::Message { sender_tid, data } => {
            ctx.send_msg(sender_tid, b"pong").ok();
        }
        _ => {}
    }
}
```

---

## VfsClient API

Lazy-initialized on first `ctx.vfs()` call. Implements the common filesystem operations:

```rust
// Read entire file
ctx.vfs().read_file("/path/to/file")?
    // → Result<Vec<u8>, ViError>

// Write entire file (creates or truncates)
ctx.vfs().write_file("/path/to/file", b"content")?
    // → Result<(), ViError>

// Stat: (size, is_dir)
let (size, is_dir) = ctx.vfs().stat("/path")?
    // → Result<(usize, bool), ViError>

// List directory
ctx.vfs().list_dir("/path")?
    // → Result<Vec<String>, ViError>

// Delete (unlink)
ctx.vfs().unlink("/path")?
    // → Result<(), ViError>

// Mkdir
ctx.vfs().mkdir("/path")?
    // → Result<(), ViError>
```

All operations are **synchronous**; the VFS service handles buffering and caching.

---

## NetClient API

Network stack exposes a **TcpStream** implementing `embedded_io::Read` + `Write`:

```rust
use embedded_io::{Read, Write};

let mut stream = ctx.net().tcp_connect(&[10, 0, 2, 2], 8080)?;
    // → Result<TcpStream, ViError>

// Write (implements embedded_io::Write)
stream.write_all(b"GET / HTTP/1.1\r\n")?;

// Read (implements embedded_io::Read)
let mut buf = [0u8; 256];
let n = stream.read(&mut buf)?;
    // → Result<usize, ViError>

let response = &buf[..n];
```

The `TcpStream` is dropped automatically (socket close on Drop).

---

## IPC & Message Sending

Send App SDK–wrapped messages:

```rust
// Send typed message to another Cell (by TID)
let remote_tid = 5usize;
ctx.send_msg(remote_tid, b"hello")?;
    // Wraps [0xAC, 0x00, b"hello"] for AppContext on the other end

// Send raw bytes (legacy)
ctx.send(remote_tid, &my_bytes)?;

// Receive (handled by app_entry! loop — you get AppEvent::Message)
```

---

## Service Discovery

Look up well-known services by ID:

```rust
use api::service;

let vfs_tid = ctx.lookup_service(service::VFS)?
    .ok_or(ViError::IO)?;
    // → Option<usize>

let net_tid = ctx.lookup_service(service::NET)?
    .ok_or(ViError::IO)?;
```

Service IDs are defined in `libs/api/src/service.rs`. Modern code should use `ctx.vfs()` and `ctx.net()` instead of manual lookup.

---

## Manifest & Syscalls

Declare what you need:

```rust
api::declare_manifest!(
    block_io = false,   // false — use VFS instead
    network = true,     // true if you use ctx.net()
    spawn = false,      // leave false unless you're init/shell
    gpio = false
);

api::declare_syscalls![Send, Recv, Log, Exit, LookupService];
```

---

## Input Events (Optional)

For UI apps:

```rust
// At startup, request input focus
ctx.request_input_focus();

// In your event loop, handle:
AppEvent::Input(input_event) => {
    // input_event is an api::input::InputEvent
    // (keyboard key / mouse move / button)
    match input_event {
        api::input::InputEvent::Key { key, pressed } => { /* ... */ }
        api::input::InputEvent::Motion { x, y } => { /* ... */ }
        api::input::InputEvent::Button { button, pressed } => { /* ... */ }
        _ => {}
    }
}
```

Alternatively, use **[Tier 1 + ViUI](viui-guide.md)** for a higher-level UI framework.

---

## Timeout & Heartbeat

Run the loop with a deadline:

```rust
// Timeout (fires AppEvent::Timeout every N ticks; 1 tick ≈ 10 ms)
ctx.run_with_timeout(1000, |ctx, event| {
    match event {
        AppEvent::Timeout => {
            // Do periodic work here
        }
        _ => {}
    }
});

// Or: arm the watchdog heartbeat
ctx.arm_heartbeat(1000);  // kernel kills us if we don't call any syscall in 1000 ticks
```

---

## Canonical Example

See [cells/apps/sdk-demo/src/main.rs](../../cells/apps/sdk-demo/src/main.rs) — 64 lines. It demonstrates VFS stat, message echo, and graceful shutdown.

---

## When to Use Tier 1 + SDK L1

✅ Reading/writing files (VFS)  
✅ Network apps (TCP, HTTP)  
✅ Talking to other Cells (IPC)  
✅ Most user applications  

❌ Complex UIs → use Tier 1 + ViUI  
❌ Cryptographic keys → use Tier 1 Extended (Silo, G2+)  
❌ C/C++ interop → use Tier 1b C/Zig  

---

## Common Errors

**VFS not registered?** —  The service may not be running. Check kernel boot output and catch `Err(_)` gracefully.

**Network port refused?** — Network service or target unreachable. Use `Result::ok()` to ignore.

**Message send fails?** — Remote Cell dead or not receiving. Use `Result::ok()` to drop silently.

---

## Next Steps

- Building a UI? → [Tier 1 + ViUI](viui-guide.md)
- Need cryptographic isolation? → [Tier 1 Extended (Silo)](tier1-silo.md)
- Have existing C code? → [Tier 1b C/Zig](tier1b-c-zig.md)
- See [api-reference.md](../api-reference.md) for syscall details.
