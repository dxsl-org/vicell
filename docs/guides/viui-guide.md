# Tier 1 Rust + ViUI — Building GUIs

> Modern, reactive UI toolkit for Cellos apps. Signal-based architecture with a compile-time-checked DSL.

---

## Overview

ViUI is a **no_std Signal-based UI framework** designed for Cellos's constraints:

- **Reactive signals**: `Signal<T>` observables for state management
- **.vi DSL**: compile-time-checked UI layout (similar to JSX)
- **Damage-driven rendering**: only redraw changed regions
- **Compositor integration**: surfaces backed by kernel grants (zero-copy)
- **Embedded performance**: no GC, WASM-ready architecture

**Target**: G2+ (planning; G1 uses basic turtle graphics via VFrame).

---

## Architecture

See [system-architecture.md § ViUI Architecture](../system-architecture.md#viui-architecture-g2-target) for:
- Signal<T> reactive model
- Layout node tree
- DirtyRect damage tracking
- ViRenderer trait
- Widget library structure

---

## Entry Point

```rust
#![no_std]
#![no_main]

extern crate alloc;

use ostd::app::{AppContext, AppEvent};
use viui::prelude::*;

ostd::app_entry!(handler = ui_handler);

fn ui_handler(ctx: &mut AppContext, event: AppEvent) {
    match event {
        AppEvent::Init => {
            // Initialize UI state here
            ctx.request_input_focus();
        }
        AppEvent::Input(input_event) => {
            // Handle keyboard/mouse
        }
        _ => {}
    }
}
```

---

## Signals & Reactivity

State is declared as a reactive signal:

```rust
use viui::signal::Signal;

// Mutable signal (cell state)
let count: Signal<i32> = Signal::new(0);

// Derived signal (computed)
let doubled: Signal<i32> = count.map(|x| x * 2);

// Update the signal (notifies subscribers)
count.set(count.get() + 1);

// Read the value
println!("{}", count.get());
```

---

## .vi DSL

Define UI layout in a compile-time-checked domain-specific language:

```rust
use viui::macros::vi_design;

let ui = vi_design! {
    <Column spacing=10>
        <Text text="Counter: {count}"/>
        <Button label="Increment" on_click={handle_increment}/>
        <Button label="Decrement" on_click={handle_decrement}/>
    </Column>
};
```

The DSL is validated at compile time; typos and missing handlers are caught before build.

---

## Compositor Integration

ViUI surfaces are backed by kernel **grants** (shared memory buffers):

```rust
// Request a surface from the compositor
let surface = ctx.request_surface(640, 480)?;

// Render to the surface (damage-driven)
let dirty_regions = surface.compute_damage();
for region in dirty_regions {
    surface.render(&ui, region);
}

// Compositor blits the grant to the screen
```

No IPC per pixel; the grant is filled once and reused.

---

## Canonical Example

See [cells/apps/robot-dashboard/src/main.rs](../../cells/apps/robot-dashboard/src/main.rs) — a working ViUI app:
- Signal state for robot telemetry
- .vi layout for the dashboard
- Compositor surface integration
- Input event handling

---

## Manifest & Syscalls

```rust
api::declare_manifest!(
    block_io = false,
    network = false,
    spawn = false
    // (GPU/DMA added automatically if viui features enabled)
);

api::declare_syscalls![Send, Recv, Log, Exit, LookupService];
```

---

## Performance

- **Startup**: <100 ms (no layouting on init)
- **Frame time**: <16 ms (60 FPS on native CPU; QEMU ~30 ms due to emulation)
- **Memory**: ~50 KiB widget tree + allocations

---

## When to Use Tier 1 + ViUI

✅ Dashboards, monitoring UIs, control panels  
✅ Embedded GUIs (robot, drone, terminal UI)  
✅ Real-time data visualization  
✅ Learn modern reactive UI patterns  

❌ Complex web-like apps → port to Tier 3b Linux + web framework  
❌ Need platform themes → design custom theme for ViUI  

---

## Next Steps

- See [system-architecture.md § ViUI Architecture](../system-architecture.md#viui-architecture-g2-target) for internals.
- Read the Signal API docs: `libs/viui-core/src/signal.rs`
- DSL syntax guide: `libs/viui-macros/` (proc_macro attributes)
- Example apps: `cells/apps/robot-dashboard/`, `cells/apps/sensor-demo/`

---

## Troubleshooting

**Compositor not found?**  
→ Start the compositor cell first: `compositor` from shell.

**DSL parse error?**  
→ Check .vi syntax. Typos in element names or attributes fail at compile time (expected).

**Surface allocation fails?**  
→ Compositor may not have enough grant memory. Reduce surface size or check kernel quota.
