# Phase 01 — wasmi Integration into WASM Driver Crate

**Status**: 📋 PLANNED  
**Priority**: P1  
**Effort**: 5 days

---

## Context Links

- WASM driver stub: `cells/drivers/wasm/src/lib.rs` — all `todo!()`
- WASM driver Cargo.toml: `cells/drivers/wasm/Cargo.toml`
- wasmi docs: https://docs.rs/wasmi/latest/wasmi/
- wasmi no_std config: `default-features = false, features = ["prefer-btree-collections"]`

---

## Overview

The `cells/drivers/wasm/` crate exists as a stub. This phase adds `wasmi v1` as a dependency and implements the core module loading and execution engine that all WASM cells will use.

Key design: the driver is a **library crate** (`lib.rs`), not a binary. It is `#![no_std]` + `extern crate alloc`. The Tier 1 host cell (Phase 03) links against it.

---

## Requirements

- `wasmi` must compile for `riscv64gc-unknown-none-elf` with `no_std`
- Module validation fails on malformed WASM (not silently ignored)
- Fuel metering enabled per-instance — prevents runaway WASM loops from starving the scheduler
- Linear memory bounded to configurable max (default 1 MiB = 16 WASM pages)
- Host functions registered via `WasmRuntime::register_vi_imports(linker)` — callers inject their `sys_send`/`sys_recv` implementations

---

## Related Code Files

### Modify
- `cells/drivers/wasm/Cargo.toml` — add wasmi, remove placeholder deps
- `cells/drivers/wasm/src/lib.rs` — implement WasmRuntime struct

---

## Implementation Steps

### Step 1 — Update `Cargo.toml`

```toml
[package]
name = "driver-wasm"
version = "0.1.0"
edition = "2021"

[dependencies]
wasmi = { version = "1", default-features = false, features = ["prefer-btree-collections"] }
types = { path = "../../libs/types" }
api   = { path = "../../libs/api" }
ostd  = { path = "../../libs/ostd" }
```

`prefer-btree-collections` disables wasmi's hashmap (needs randomness) in favour of BTreeMap — required for `no_std`.

### Step 2 — Implement `WasmRuntime` in `lib.rs`

```rust
#![no_std]
extern crate alloc;

use alloc::vec::Vec;
use wasmi::{Engine, Linker, Module, Store, StackLimits};

/// Configuration for a WASM cell instance.
pub struct WasmConfig {
    /// Maximum fuel (instructions) per scheduler tick before a yield.
    /// ~100_000 corresponds to ~1 ms of interpreter work at typical speeds.
    pub fuel_per_tick: u64,
    /// Maximum WASM linear memory in bytes (default 1 MiB = 16 × 64KB pages).
    pub max_memory_bytes: usize,
}

impl Default for WasmConfig {
    fn default() -> Self {
        Self { fuel_per_tick: 100_000, max_memory_bytes: 1024 * 1024 }
    }
}

/// State visible to WASM host import functions.
pub struct HostState {
    /// Kernel task ID of the cell running this WASM instance.
    pub cell_id: usize,
}

/// Lifecycle container for a running WASM module.
pub struct WasmRuntime {
    engine: Engine,
}

impl WasmRuntime {
    /// Initialise the wasmi engine with fuel metering and conservative stack limits.
    pub fn new(config: &WasmConfig) -> Self {
        let mut engine_config = wasmi::Config::default();
        engine_config.consume_fuel(true);
        // Limit interpreter stack depth to prevent runaway recursion.
        engine_config.set_stack_limits(
            StackLimits::new(32 * 1024, 64 * 1024, 512).expect("valid stack limits"),
        );
        Self { engine: Engine::new(&engine_config) }
    }

    /// Parse and validate a WASM binary.  Returns an error on malformed input.
    pub fn load_module(&self, wasm_bytes: &[u8]) -> Result<Module, wasmi::Error> {
        Module::new(&self.engine, wasm_bytes)
    }

    /// Create a `Store` with initial fuel and host state.
    pub fn new_store(&self, config: &WasmConfig, host: HostState) -> Store<HostState> {
        let mut store = Store::new(&self.engine, host);
        store.set_fuel(config.fuel_per_tick).expect("fuel metering enabled");
        store
    }

    /// Provide a pre-configured `Linker` ready for `vi.*` import registration.
    pub fn new_linker(&self) -> Linker<HostState> {
        Linker::new(&self.engine)
    }
}
```

### Step 3 — Verify compiles for RISC-V target

Run `cargo check --target riscv64gc-unknown-none-elf -Z build-std=core,alloc -p driver-wasm` to confirm no `std` dependency is pulled in transitively.

---

## Todo List

- [ ] Update `cells/drivers/wasm/Cargo.toml` — add wasmi + prefer-btree-collections
- [ ] Implement `WasmConfig`, `HostState`, `WasmRuntime` in `lib.rs`
- [ ] `cargo check --target riscv64gc-unknown-none-elf -Z build-std=core,alloc -p driver-wasm` — clean
- [ ] Verify wasmi `prefer-btree-collections` disables all hash-based code paths

---

## Success Criteria

- [ ] `driver-wasm` compiles for `riscv64gc-unknown-none-elf` target with zero errors
- [ ] `WasmRuntime::load_module()` returns `Err` for truncated WASM binary
- [ ] Fuel metering configured — store cannot execute indefinitely without refuel

---

## Risk Assessment

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| wasmi pulls in `std` transitively | Low | `prefer-btree-collections` + `default-features=false` tested on ESP32-C6 |
| Stack limits reject valid WASM programs | Low | Start conservative (64KB), increase if integration tests fail |
| wasmi binary size too large for cell heap | Low | Estimate 300–600 KB; within 4 MiB cell quota |
