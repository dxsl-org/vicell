# Error Handling Patterns
> Part of [Cellos Patterns](../patterns.md)

## Result-Based Error Handling

**Intent**: Use `Result<T, E>` for all recoverable errors.

```rust
// libs/types/src/lib.rs
pub type ViResult<T> = Result<T, ViError>;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViError {
    NotFound,
    OutOfMemory,
    InvalidArgument,
    PermissionDenied,
    IoError,
    // ...
}
```

**Early return with `?`**:
```rust
fn complex_operation() -> ViResult<()> {
    let resource1 = acquire_resource1()?;  // propagates error immediately
    let resource2 = acquire_resource2()?;
    process(resource1, resource2)?;
    Ok(())
}
```

## Error Context

**Intent**: Log context before propagating — callers get the error code, logs get the detail.

```rust
fn load_config() -> ViResult<Config> {
    let data = fs::read_file("/etc/config.toml")
        .map_err(|e| {
            log::error!("Failed to read config: {:?}", e);
            ViError::ConfigLoadFailed
        })?;

    parse_toml(&data).map_err(|e| {
        log::error!("Failed to parse config: {:?}", e);
        ViError::ConfigParseFailed
    })
}
```

## Panic for Unrecoverable Errors

**Intent**: `panic!` for programming bugs and kernel invariant violations only.
Cells should use `Result` whenever possible; `panic` is a last resort.

```rust
pub fn schedule_next() {
    let mut sched = SCHEDULER.lock();
    let scheduler = sched.as_mut()
        .expect("Scheduler not initialized — init() must be called first");

    scheduler.pick_next_task()
        .expect("No runnable tasks — idle task must always exist");
}
```

**Three-level model** (from Midori/Rust design):
| Level | Mechanism | When |
|-------|-----------|------|
| Expected error | `Result<T, E>` | Caller must handle (I/O, network, parse) |
| Contract violation | `expect("why")` | Bug in calling code, debug builds |
| Kernel invariant | `panic!` | Hardware failure, impossible state |
