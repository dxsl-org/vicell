# Tier 1 Rust (Bare) — Minimal Cell Apps

> Direct syscall access, no SDK abstractions. For apps that are pure compute or need fine-grained control.

---

## Entry Point: `ostd::app_entry!`

Instead of `#[no_mangle] fn main()`, use the zero-boilerplate macro:

```rust
#![no_std]
#![no_main]

use ostd::app::{AppContext, AppEvent};
use ostd::io::println;

ostd::app_entry!(handler = my_handler);

fn my_handler(_ctx: &mut AppContext, event: AppEvent) {
    match event {
        AppEvent::Init => println("Hello from ViCell!"),
        AppEvent::Shutdown | AppEvent::ShutdownWith { .. } => {
            ostd::syscall::sys_exit(0);
        }
        _ => {}
    }
}
```

The macro generates:
- Manifest declaration (empty/default capabilities)
- Syscall allowlist (Init, Shutdown, Log, Exit only)
- `#[no_mangle] pub fn main()` wrapper
- `unsafe` isolation (you're safe)

---

## AppEvent Variants

| Event | When | Handler Duty |
|-------|------|--------------|
| `Init` | Before first `sys_recv` (if using `app_entry!` + `run_with_lifecycle`) | Startup: config, logging setup, resource init. |
| `Message { sender_tid, data }` | Typed IPC message arrived (envelope starts `0xAC`) | Echo, relay, or dispatch. |
| `RawMessage { sender_tid, data }` | Raw `sys_send` (legacy/non-SDK senders) | Ignore or handle specially. |
| `Input(InputEvent)` | Keyboard/mouse input (only if `request_input_focus()` called) | UI apps only. |
| `Timeout` | Receive deadline elapsed (only `run_with_timeout()`) | Periodic tasks, watchdog. |
| `Shutdown` | Kernel graceful shutdown (no reason) | Exit cleanly. |
| `ShutdownWith { reason }` | Kernel shutdown with reason (Watchdog / ParentDied / Requested) | Restart vs. abort logic. |

Always add `_ => {}` wildcard to future-proof your code.

---

## Manifest & Syscall Allowlist

Declare capabilities and permitted syscalls:

```rust
api::declare_manifest!(
    block_io = false,   // raw disk I/O
    network = false,    // network access
    spawn = false,      // spawn other Cells
    gpio = false,       // GPIO peripherals
    uart = false,       // UART serial
    hypervisor = false  // create VMs (Tier 3b, G2+)
);

api::declare_syscalls![Send, Recv, Log, Exit, GetTime];
```

**Capabilities** are kernel grants (honored only for `/bin/*` binaries). **Syscalls** are the thin whitelist the kernel enforces. Omit what you don't use.

---

## Syscall Allowlist

Common syscalls:
- `Init` — initialization (implicit in app_entry!)
- `Send`, `Recv` — IPC
- `Log` — `println!`
- `Exit` — `sys_exit()`
- `GetTime` — `sys_get_time()`
- `Heartbeat` — `sys_heartbeat()` (watchdog)
- `LookupService` — `sys_lookup_service()`
- `GetRandom` — entropy

See [api-reference.md](../api-reference.md) for the full list.

---

## Minimal Example

```rust
#![no_std]
#![no_main]

use ostd::io::println;

api::declare_manifest!(block_io = false, network = false, spawn = false);
api::declare_syscalls![Log, Exit];

ostd::app_entry!(handler = main_handler);

fn main_handler(_ctx: &mut AppContext, event: ostd::app::AppEvent) {
    match event {
        ostd::app::AppEvent::Init => {
            println("Cell started");
        }
        _ => {}
    }
}
```

That's it. No IPC, no service clients, no async — just init and exit.

---

## When to Use Tier 1 Bare

✅ Numeric computation, data processing  
✅ Pure-Rust no external I/O  
✅ Extreme performance requirements  
✅ Learning syscalls directly  

❌ Reading files → use Tier 1 + SDK L1 (VFS client)  
❌ Talking to other Cells → use Tier 1 + SDK L1 (IPC wrappers)  
❌ Building UIs → use Tier 1 + ViUI  

---

## Canonical Example

See [cells/apps/hello-cell/src/main.rs](../../cells/apps/hello-cell/src/main.rs) — 18 lines total.

---

## Next Steps

- Need services (VFS, network)? → [Tier 1 + SDK L1](tier1-rust-sdk.md)
- Need a UI? → [Tier 1 + ViUI](viui-guide.md)
- Need C interop? → [Tier 1b C/Zig](tier1b-c-zig.md)
