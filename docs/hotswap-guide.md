# ViOS Hot-Swap Guide

Live-replace a running Cell with a new version without losing session state.

---

## How It Works

```
1. Kernel freezes the old Cell (queues incoming IPC, stops scheduling it)
2. Kernel calls Cell's serialize_state() → owned byte buffer
3. Kernel loads new ELF from disk via SpawnFromPath
4. Kernel calls new Cell's deserialize_state(buffer)
5. Kernel unfreezes (new Cell receives queued messages)
```

No messages are lost during the swap.  The user session continues from where
it was before the upgrade.

---

## Implementing `ViStateTransfer`

Every Cell that wants to support hot-swap must implement
`api::hotswap::ViStateTransfer`:

```rust
use api::hotswap::ViStateTransfer;
use api::prelude::*;

const SCHEMA_VERSION: u32 = 1; // bump on incompatible changes

struct MyCell {
    counter: u64,
    name: alloc::string::String,
}

impl ViStateTransfer for MyCell {
    fn state_size(&self) -> usize {
        4 + 8 + 2 + self.name.len()   // version + counter + name_len + name
    }

    fn serialize_state(&self, buf: &mut [u8]) -> ViResult<usize> {
        let needed = self.state_size();
        if buf.len() < needed { return Err(ViError::InvalidArgument); }
        let mut pos = 0;
        buf[pos..pos+4].copy_from_slice(&SCHEMA_VERSION.to_le_bytes()); pos += 4;
        buf[pos..pos+8].copy_from_slice(&self.counter.to_le_bytes());   pos += 8;
        let nl = self.name.len() as u16;
        buf[pos..pos+2].copy_from_slice(&nl.to_le_bytes());             pos += 2;
        buf[pos..pos+self.name.len()].copy_from_slice(self.name.as_bytes());
        pos += self.name.len();
        Ok(pos)
    }

    fn deserialize_state(&mut self, buf: &[u8]) -> ViResult<()> {
        if buf.len() < 14 { return Err(ViError::InvalidInput); }
        let _version  = u32::from_le_bytes([buf[0],buf[1],buf[2],buf[3]]);
        self.counter  = u64::from_le_bytes(buf[4..12].try_into().unwrap());
        let nl        = u16::from_le_bytes([buf[12],buf[13]]) as usize;
        self.name     = core::str::from_utf8(&buf[14..14+nl])
                            .map_err(|_| ViError::InvalidInput)?
                            .into();
        Ok(())
    }
}
```

---

## Schema Versioning Rules

- **Bump `SCHEMA_VERSION`** whenever the binary layout changes (field added,
  removed, or reordered).
- `deserialize_state` must handle both old and new schema versions gracefully.
  If the version is unrecognised, return `Err(ViError::InvalidInput)` — the
  kernel will abort the swap and keep the old Cell running.
- Adding new optional fields at the **end** of the payload is safe with a
  bounds-check on `buf.len()`.  Removing or reordering fields requires a version bump.

---

## What Can and Cannot Survive a Hot-Swap

| State | Survives? | Notes |
|-------|-----------|-------|
| In-memory data (counters, KV map, history) | ✅ Yes | Via `serialize_state` |
| IPC messages in flight | ✅ Yes | Kernel queues them during freeze |
| Open file descriptors (POSIX) | ❌ No | POSIX shim re-opens on new Cell startup |
| Capability handles (`ViFileHandle`) | ⚠️ Partial | VFS cell must re-issue caps to new owner |
| Pending async futures | ❌ No | Async executor state is not serialisable in v1.0 |
| Hardware IRQ state | ❌ No | Driver Cells cannot currently be hot-swapped |

---

## Triggering a Hot-Swap

The `hotswap` admin tool (`cells/sys-tools/src/hotswap.rs`) is implemented:

```
ViOS> hotswap config /bin/config-v2
[hotswap] freezing cell 'config' (id 3)...
[hotswap] serialised 142 bytes of state
[hotswap] loaded /bin/config-v2
[hotswap] deserialised state into new cell
[hotswap] unfrozen — 3 queued messages delivered
[hotswap] done in 12ms
```

Alternatively, trigger via the `HotSwap` syscall (id 400) from any privileged cell.

---

## Implemented Cells

| Cell | State Serialised | Schema Version | Status |
|------|-----------------|----------------|--------|
| Config | KV map | v1 | ✅ Verified |
| Shell | History + aliases | v1 | ✅ Verified |
| VFS | Open handle table | v1 | ✅ Partial (handle serialization) |

---

## Grant Chains

When one Cell delegates a capability to another (`grant_to`), the new cap's
`grant_depth` is decremented.  A cap with `grant_depth == 0` cannot be
delegated further — any attempt returns `ViError::NotSupported`.

Default depth: **4** (`MAX_GRANT_DEPTH` in `kernel/src/cell/cap_registry.rs`).
Change this per-cap with `alloc_with_grant_depth()` (planned helper).

This prevents unbounded delegation chains that could create capability cycles.

---

## Lease Auto-Revocation

A cap with `expires_at` set is silently revoked on the next `verify()` call
after the deadline passes.  The caller receives `ViError::PermissionDenied`.

```rust
// Example: allocate a cap that expires after 1 second (10M ticks at 10 MHz)
let now = crate::task::system_ticks() as u64;
let cap = table.alloc_with_lease(owner, resource, perms, now + 10_000_000);
```

---

## Files

| File | Purpose |
|------|---------|
| `libs/api/src/hotswap.rs` | `ViStateTransfer` trait definition |
| `kernel/src/cell/cap_registry.rs` | Grant depth + lease expiry enforcement |
| `kernel/src/cell/hotswap.rs` | HotSwap orchestrator (5-step protocol) |
| `cells/services/config/src/main.rs` | Config KV state transfer impl |
| `cells/services/vfs/src/state_transfer.rs` | VFS handle table state transfer |
| `cells/apps/shell/src/state_transfer.rs` | Shell history + alias state transfer |
| `cells/sys-tools/src/hotswap.rs` | HotSwap CLI tool |
