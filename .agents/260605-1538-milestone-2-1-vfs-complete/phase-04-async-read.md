# Phase 04 — Non-Blocking Async Read Protocol

**Status**: 📋 PLANNED  
**Priority**: P2  
**Effort**: 5 days

---

## Context Links

- VFS main loop: `cells/services/vfs/src/main.rs:450-583` — synchronous `sys_recv` loop
- `VfsRequest`/`VfsResponse` enums: `libs/api/src/ipc.rs:23-60`
- Block I/O syscalls: `libs/ostd/src/syscall.rs` — `sys_blk_read()` (synchronous)
- Current `GetFile` handler: `cells/services/vfs/src/main.rs:495-500` — synchronous fast path
- `sys_recv` returns sender immediately on message receipt

---

## Design: Two-Opcode Non-Blocking Protocol

**Research finding**: Full Rust async requires a waker-ready block driver. The VirtIO block driver is currently synchronous. The minimal viable async path uses a two-opcode protocol:

1. `ReadAsync { path }` → VFS starts the read, returns `PendingHandle(u32)` immediately
2. `Poll { handle: u32 }` → VFS checks if read is complete, returns `Data(...)` or `Pending`

**Caller loop:**
```rust
let handle = match vfs_read_async(path) {
    VfsResponse::PendingHandle(h) => h,
    VfsResponse::Data(bytes) => return Ok(bytes), // fast path
    _ => return Err(...)
};
loop {
    match vfs_poll(handle) {
        VfsResponse::Data(bytes) => return Ok(bytes),
        VfsResponse::Pending     => task::yield_now(),
        _                        => return Err(...)
    }
}
```

**VFS implementation**: Since `sys_blk_read` is still synchronous, the VFS issues the disk read immediately on `ReadAsync` but returns `PendingHandle` anyway. On the first `Poll`, the data is already ready (it was read during `ReadAsync`). This makes the protocol correct without actually overlapping I/O — the benefit comes later when the block driver gains interrupt-driven async.

---

## New IPC Opcodes

```rust
// In libs/api/src/ipc.rs — VfsRequest additions:
/// Start a non-blocking file read. Returns PendingHandle immediately.
/// For large files (>512 bytes), caller must Poll to retrieve remaining chunks.
ReadAsync { path: &'a str, offset: u64, len: u32 },

/// Poll a pending read handle for completion.
Poll { handle: u32 },

// VfsResponse additions:
/// Async read accepted; poll this handle for data.
PendingHandle(u32),
/// Read in progress (poll again after yield_now).
Pending,
```

---

## Implementation Steps

### Step 1 — Add `PendingTable` to VFS

```rust
// cells/services/vfs/src/pending.rs (new file)
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

pub struct PendingRead {
    pub data:   Vec<u8>,      // pre-read data (synchronous backend)
    pub offset: usize,        // current read cursor
}

pub struct PendingTable {
    table:  BTreeMap<u32, PendingRead>,
    next_id: u32,
}

impl PendingTable {
    pub fn new() -> Self { Self { table: BTreeMap::new(), next_id: 1 } }

    pub fn insert(&mut self, data: Vec<u8>) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.table.insert(id, PendingRead { data, offset: 0 });
        id
    }

    /// Returns the data if ready (always ready in sync backend), removes the handle.
    pub fn poll(&mut self, handle: u32) -> Option<Vec<u8>> {
        self.table.remove(&handle).map(|p| p.data)
    }
}
```

### Step 2 — Handle `ReadAsync` and `Poll` in VFS main loop

```rust
// Add to GLOBAL_VFS: pending: PendingTable

// ReadAsync handler:
VfsRequest::ReadAsync { path, offset: _, len: _ } => {
    // Read synchronously (async backend future work)
    let data = read_file_bytes(fat_fs.as_ref(), &mut vfs, path);
    let handle = gvfs.pending.insert(data);
    api::ipc::VfsResponse::PendingHandle(handle)
}

// Poll handler:
VfsRequest::Poll { handle } => {
    match gvfs.pending.poll(handle) {
        Some(data) => {
            // Encode data into resp_buf, send raw (may need multiple replies for large files)
            api::ipc::VfsResponse::Data(&data[..data.len().min(480)])
        }
        None => api::ipc::VfsResponse::Pending,
    }
}
```

### Step 3 — Helper: `read_file_bytes`

Extract the read logic from `GetFile` and `ReadAsync` into a shared function:

```rust
fn read_file_bytes(fat_fs: Option<&DataFs>, vfs: &VfsManager, path: &str) -> Vec<u8> {
    if path.starts_with("/data/") {
        read_fat16_to_vec(fat_fs, path)
    } else {
        vfs.get_file_data(path).map(|d| d.to_vec()).unwrap_or_default()
    }
}
```

### Step 4 — Update shell callers

Add `async_cat` shell builtin using `ReadAsync` + `Poll`:

```rust
"async-cat" => {
    let handle = vfs_read_async(path)?;
    loop {
        match vfs_poll(handle) {
            Data(bytes) => { print_bytes(&bytes); break; }
            Pending     => task::yield_now(),
        }
    }
}
```

---

## Todo List

- [ ] Add `ReadAsync` and `Poll` to `VfsRequest` enum (`libs/api/src/ipc.rs`)
- [ ] Add `PendingHandle` and `Pending` to `VfsResponse` enum
- [ ] Create `cells/services/vfs/src/pending.rs` with `PendingTable`
- [ ] Add `pending: PendingTable` to VFS global state
- [ ] Handle `ReadAsync` and `Poll` in VFS main loop
- [ ] Extract `read_file_bytes()` helper
- [ ] Add `async-cat` shell command for demonstration
- [ ] `cargo check -p service-vfs -p app-shell` — clean
- [ ] Test: `async-cat /data/large.bin` returns correct data via poll loop

---

## Success Criteria

- [ ] `ReadAsync` returns `PendingHandle(n)` immediately (< 1ms, no blocking)
- [ ] `Poll(n)` returns `Data(...)` on next call (data was pre-read)
- [ ] Shell `async-cat` command reads a file correctly via the two-opcode protocol
- [ ] Stale handle (already polled) returns `Err` not panic
- [ ] All existing VFS sync tests still pass

---

## Risk Assessment

| Risk | Mitigation |
|------|-----------|
| Data larger than 480-byte IPC buffer | Phase 04 MVP: only return first 480 bytes; future chunking with cursor |
| PendingTable grows unbounded | Add handle expiry (TTL = 10 seconds of ticks) |
| Stale handle from crashed cell | TTL expiry handles this; no cleanup protocol needed for MVP |
