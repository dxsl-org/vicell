# Phase 01 — ZST Capability Tokens

**Status**: ✅ COMPLETE (2026-06-07 audit)  
**Priority**: P0 (security hole)  
**Effort**: 3 days

---

## Context Links

- Current KernelPerms: `kernel/src/task/tcb.rs:89-111`
- BLOCK_IO grant site: `kernel/src/loader.rs:71-80`
- BLOCK_IO check: `kernel/src/task/syscall.rs:67-82`
- NetTx/NetRx (unguarded): `kernel/src/task/syscall.rs:1117-1131`
- SpawnFromPath handler: `kernel/src/task/syscall.rs:795-835`

---

## Overview

`KernelPerms(u32)` is a bitfield where bits are checked via `contains()`. Problems:
1. `NetTx`/`NetRx` are **completely unguarded** — any Cell can access the network
2. `SpawnFromPath`/`SpawnPinned` are unguarded — any Cell can spawn binaries
3. Future bit additions accumulate in an opaque integer; wrong bit means wrong privilege

Replace with three `Option<ZST>` fields on `Task`. `Option<ZST>` = 1 byte (niche optimization), so three caps = 3 bytes — smaller than the current `u32`.

---

## Security Impact

| Syscall | Current | After |
|---------|---------|-------|
| `BlkRead`/`BlkWrite`/`BlkFlush` | guarded by `BLOCK_IO` bit | `block_io_cap.is_some()` |
| `NetTx`/`NetRx` | **unguarded** | `network_cap.is_some()` |
| `SpawnFromPath`/`SpawnPinned` | **unguarded** | `spawn_cap.is_some()` |
| `HotSwap` | unguarded | `spawn_cap.is_some()` (same authority) |

**⚠️ Critical design note**: `init` cell (CellId 1) spawns all system services via `sys_spawn_from_path` syscall (`cells/apps/init/src/main.rs:21-78`). If SpawnCap is required for that syscall, init cannot boot the system. Fix: grant SpawnCap to init at kernel spawn time in `main.rs` (before `spawn_from_mem(&init_data, ...)` returns), AND to shell in `loader.rs`.

---

## Related Code Files

### Create
- `kernel/src/task/cap.rs` — ZST types + `pub(crate)` constructors

### Modify
- `kernel/src/task/tcb.rs` — replace `kernel_perms: KernelPerms` with three `Option<XxxCap>` fields
- `kernel/src/loader.rs` — replace `KernelPerms` grant with explicit cap grants per path
- `kernel/src/task/syscall.rs` — update 3 block_io checks; add 2 network guards; add spawn guards

---

## Implementation Steps

### Step 1 — Create `kernel/src/task/cap.rs`

```rust
//! Kernel-internal capability tokens.
//!
//! Each token is a zero-sized type (ZST).  Only the kernel can construct them
//! (via `pub(in crate::kernel) fn new()`).  Cell crates are separate Rust
//! compilation units and cannot name `crate::kernel::task::cap`, so forgery
//! is a compile error — no runtime check needed.
//!
//! `Option<ZST>` uses Rust's niche optimization: exactly 1 byte on the wire.
//! Three caps together are 3 bytes vs the previous `KernelPerms(u32)` = 4 bytes.

/// Permits raw block-device syscalls (BlkRead, BlkWrite, BlkFlush).
/// Granted to `/bin/vfs` at spawn.
#[derive(Copy, Clone, Debug)]
pub struct BlockIoCap(());

/// Permits network transmit and receive syscalls (NetTx, NetRx).
/// Granted to `/bin/net` at spawn.
#[derive(Copy, Clone, Debug)]
pub struct NetworkCap(());

/// Permits spawning new Cells and hot-swapping running Cells.
/// Granted to `/bin/shell` at spawn.
#[derive(Copy, Clone, Debug)]
pub struct SpawnCap(());

impl BlockIoCap {
    /// Create a `BlockIoCap` token.  Only callable within `crate::kernel`.
    pub(crate) fn new() -> Self { Self(()) }
}
impl NetworkCap {
    pub(crate) fn new() -> Self { Self(()) }
}
impl SpawnCap {
    pub(crate) fn new() -> Self { Self(()) }
}
```

Add `pub mod cap;` to `kernel/src/task.rs`.

### Step 2 — Update TCB (`tcb.rs`)

Remove `KernelPerms` field, add three `Option<XxxCap>`:

```rust
// Remove:
pub kernel_perms: KernelPerms,

// Add:
/// Raw block-device access (BlkRead/BlkWrite/BlkFlush).  Set at spawn for /bin/vfs.
pub block_io_cap: Option<cap::BlockIoCap>,
/// Network transmit/receive (NetTx/NetRx).  Set at spawn for /bin/net.
pub network_cap:  Option<cap::NetworkCap>,
/// Cell spawning and hot-swap (SpawnFromPath/SpawnPinned/HotSwap).  Set for /bin/shell.
pub spawn_cap:    Option<cap::SpawnCap>,
```

Update `Task::new()` to default all three to `None`.

### Step 3 — Update grant logic (`loader.rs` + `main.rs`)

```rust
// In loader.rs (spawn_from_path):
match path {
    p if p.ends_with("/bin/vfs")   => { task.block_io_cap = Some(cap::BlockIoCap::new()); }
    p if p.ends_with("/bin/net")   => { task.network_cap  = Some(cap::NetworkCap::new()); }
    p if p.ends_with("/bin/shell") => { task.spawn_cap    = Some(cap::SpawnCap::new()); }
    _ => {}
}

// In main.rs — grant SpawnCap to init after spawn_from_mem:
// (init uses sys_spawn_from_path to boot vfs/config/shell; without SpawnCap it cannot)
if let Some(sched) = SCHEDULER.lock().as_mut() {
    if let Some(task) = sched.tasks.get_mut(&init_tid) {
        task.spawn_cap = Some(cap::SpawnCap::new());
    }
}
```

### Step 4 — Update syscall checks

**Block I/O** (3 sites): replace `caller_has_block_io(caller_id)` with:
```rust
fn caller_has_block_io(caller_id: usize) -> bool {
    SCHEDULER.lock().as_ref()
        .and_then(|s| s.tasks.get(&caller_id))
        .map(|t| t.block_io_cap.is_some())
        .unwrap_or(false)
}
```

**Network** (2 sites — NetTx/NetRx, currently unguarded): add at the top of each handler:
```rust
if !SCHEDULER.lock().as_ref()
    .and_then(|s| s.tasks.get(&caller_id))
    .map(|t| t.network_cap.is_some())
    .unwrap_or(false)
{
    return Err(SyscallError::PermissionDenied);
}
```

**Spawn** (SpawnFromPath, SpawnPinned, HotSwap): add guard with `spawn_cap.is_some()`.

---

## Todo List

- [ ] Create `kernel/src/task/cap.rs` (BlockIoCap, NetworkCap, SpawnCap with kernel-only constructors)
- [ ] Add `pub mod cap;` to `kernel/src/task.rs`
- [ ] Replace `kernel_perms: KernelPerms` with three `Option<XxxCap>` fields in TCB
- [ ] Update `Task::new()` — all caps default to None
- [ ] Update `loader.rs` — grant caps per cell path
- [ ] Update `syscall.rs` — fix 3 block_io checks, add 2 network guards, add spawn guards
- [ ] Remove `KernelPerms` struct (no longer needed)
- [ ] `cargo check -p vicell-kernel` — zero errors
- [ ] Test: Cell without NetworkCap calling NetTx → PermissionDenied

---

## Success Criteria

- [ ] `NetTx`/`NetRx` return `PermissionDenied` when called from shell/vfs/config
- [ ] `/bin/net` can still call `NetTx`/`NetRx`
- [ ] `/bin/vfs` can still call `BlkRead`/`BlkWrite`
- [ ] All 65 existing integration tests pass (NetworkCap granted to net at spawn)
- [ ] `KernelPerms` type removed from codebase (search returns zero results)

---

## Risk Assessment

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| Forgot to grant NetworkCap to /bin/net → net service broken | Medium | Integration test net_tcp covers this |
| SpawnCap guard breaks init's ability to spawn cells | High | Init spawns via `spawn_from_path` internally (kernel code, not syscall) — no cap needed for kernel spawns |
| `pub(in crate::kernel)` visibility scope too narrow | Low | `crate::kernel` covers all of kernel/src/ — correct scope |
