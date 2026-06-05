//! WASM driver runtime for ViCell Tier 2 cells.
//!
//! Wraps `wasmi` v1 to provide a `WasmRuntime` that:
//! - Parses and validates `.wasm` binaries
//! - Runs them under wasmi's interpreter with fuel metering
//! - Bridges `vi.*` host imports to ViCell's IPC syscall layer

#![no_std]
extern crate alloc;

pub mod imports;

use wasmi::{Engine, Linker, Module, Store};

/// Configuration for a WASM cell instance.
#[derive(Clone)]
pub struct WasmConfig {
    /// Fuel units (roughly, interpreter steps) consumed per scheduler tick
    /// before `yield_cpu()` is called.  ~100_000 ≈ 1 ms at typical wasmi speed.
    pub fuel_per_tick: u64,
}

impl Default for WasmConfig {
    fn default() -> Self {
        Self { fuel_per_tick: 100_000 }
    }
}

/// State accessible to WASM host import functions via `Caller<HostState>`.
pub struct HostState {
    /// Kernel task ID of the cell hosting this WASM instance.
    /// Used by `vi.send` / `vi.recv` to identify the caller in IPC.
    pub cell_task_id: usize,
}

/// Lifecycle container for a running WASM module.
///
/// Create once per WASM binary; reuse the engine for multiple stores
/// if running several instances of the same module.
pub struct WasmRuntime {
    engine: Engine,
}

impl WasmRuntime {
    /// Initialise the wasmi engine with fuel metering enabled.
    ///
    /// Fuel metering is required so the scheduler can preempt a
    /// long-running WASM loop by checking for `OutOfFuel` on each
    /// interpreter step.
    pub fn new(_config: &WasmConfig) -> Self {
        let mut engine_cfg = wasmi::Config::default();
        engine_cfg.consume_fuel(true);
        Self { engine: Engine::new(&engine_cfg) }
    }

    /// Parse and validate a WASM binary.
    ///
    /// Returns `Err` if the binary is malformed, invalid, or uses
    /// unsupported proposals.  Always call this before `new_store`.
    pub fn load_module(&self, wasm_bytes: &[u8]) -> Result<Module, wasmi::Error> {
        Module::new(&self.engine, wasm_bytes)
    }

    /// Create a `Store` with initial fuel and the given host state.
    ///
    /// Each WASM instance needs its own `Store`.  The fuel limit controls
    /// how many interpreter steps run before the host sees `OutOfFuel`.
    pub fn new_store(&self, config: &WasmConfig, host: HostState) -> Store<HostState> {
        let mut store = Store::new(&self.engine, host);
        store.set_fuel(config.fuel_per_tick)
            .expect("fuel metering is always enabled in WasmRuntime::new");
        store
    }

    /// Create a `Linker` ready for `vi.*` import registration.
    ///
    /// Callers should call `imports::register_vi_imports(linker)` before
    /// instantiating a module.
    pub fn new_linker(&self) -> Linker<HostState> {
        Linker::new(&self.engine)
    }
}
