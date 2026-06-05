# Phase 01 — Typed IPC Message Enums

**Status**: 📋 PLANNED  
**Priority**: P1  
**Effort**: 4 days

---

## ⚠️ Law 1 Gate — 2x Confirmation Required

Adding `libs/api/src/ipc.rs` with new public types. These are new API additions (no removals), so existing Cells compile without changes. Confirmation needed per Law 1.

**Proposed additions:**
- `libs/api/src/ipc.rs` — typed request/response enums (VfsRequest, VfsResponse, NetRequest, NetResponse, etc.)
- `postcard` dependency in `libs/api/Cargo.toml`

---

## Context Links

- Current VFS wire format: `cells/services/vfs/src/main.rs:32-41` — 10 u8 opcodes + manual byte layout
- IPC syscall: `libs/ostd/src/syscall.rs:18-36` — raw `(ptr, len)` pairs
- IPC dispatch: `kernel/src/task.rs` — `ipc_send()`, `ipc_recv()`
- Current buffer size: unbounded (MAX_USER_BUF = 64 MiB); target = 512 bytes per message

---

## Overview

Currently all IPC messages are raw byte slices. The VFS service parses opcodes manually at byte offsets (e.g., `[path_len:u8][content_len:u16 LE][path][content]`). This is fragile: both sides must maintain identical undocumented byte math; there's no type safety at the call site.

**Strategy**: Use `postcard` (no_std, serde-based) to serialize Rust enums into a fixed `[u8; 512]` stack buffer. The existing `sys_send(target, ptr, len)` ABI is unchanged — postcard just fills the buffer. Migration is additive: old byte-opcode VFS protocol remains until the VFS service is updated.

---

## Requirements

- `postcard::to_slice` + `from_bytes` work in no_std + no_alloc (confirmed from research)
- Enum discriminant is 1 byte (varint for < 127 variants) — overhead is negligible
- `[u8; 512]` buffer keeps existing IPC buffer size; `MaxSize` compile-time check ensures fit
- VFS migration: old `OP_*` u8 opcodes must coexist until VFS is fully migrated (version-gate)
- No change to `sys_send()` / `sys_recv()` signatures

---

## Architecture

```
Cell (sender)                           VFS service (receiver)
  VfsRequest::Read { path }
  ─── postcard::to_slice(&req, &mut buf) ──→ [0x01, 0x05, 'h','e','l','l','o', ...]
  sys_send(VFS_TASK_ID, buf.as_ptr(), used_len)
                                            sys_recv(MASK_ALL, &mut recv_buf)
                                            postcard::from_bytes(&recv_buf) → VfsRequest::Read { "hello" }
```

---

## Related Code Files

### Create
- `libs/api/src/ipc.rs` — typed message enums (VfsRequest, VfsResponse, NetRequest, NetResponse)

### Modify
- `libs/api/Cargo.toml` — add `postcard = { version = "1", default-features = false, features = ["derive"] }`
- `libs/api/src/lib.rs` — add `pub mod ipc`
- `cells/services/vfs/src/main.rs` — migrate from u8 opcodes to VfsRequest enum
- `libs/ostd/src/lib.rs` — re-export `api::ipc` for convenience

---

## Implementation Steps

### Step 1 — Add `postcard` dependency

```toml
# libs/api/Cargo.toml
[dependencies]
postcard = { version = "1", default-features = false, features = ["derive"] }
serde = { version = "1", default-features = false, features = ["derive"] }
```

### Step 2 — Create `libs/api/src/ipc.rs`

```rust
//! Typed IPC message enums for ViCell services.
//!
//! Both kernel and Cell crates link to `libs/api`, so these types are shared
//! across the IPC boundary without any unsafe casting.
//!
//! Serialization: `postcard::to_slice` writes into a caller-provided `[u8; 512]`
//! stack buffer.  `postcard::from_bytes` deserializes on the receiver side.
//! The wire representation is compact (1-byte discriminant + varint lengths).

use serde::{Deserialize, Serialize};

/// Maximum IPC payload size (matches existing IPC buffer convention).
pub const IPC_BUF_SIZE: usize = 512;

// ── VFS service ───────────────────────────────────────────────────────────────

/// Requests understood by the VFS service (`/bin/vfs`).
#[derive(Debug, Serialize, Deserialize)]
pub enum VfsRequest<'a> {
    /// Read the entire contents of a file.
    GetFile  { path: &'a str },
    /// List directory entries as a newline-separated string.
    ListDir  { path: &'a str },
    /// Stat a path — returns Ok or Err.
    Stat     { path: &'a str },
    /// Write (create/overwrite) a file.
    Write    { path: &'a str, content: &'a [u8] },
    /// Append to a file.
    Append   { path: &'a str, content: &'a [u8] },
    /// Create a directory.
    Mkdir    { path: &'a str },
    /// Remove an empty directory.
    Rmdir    { path: &'a str },
    /// Delete a file.
    Unlink   { path: &'a str },
}

/// Responses from the VFS service.
#[derive(Debug, Serialize, Deserialize)]
pub enum VfsResponse<'a> {
    /// Successful read — raw file bytes.
    Data   (&'a [u8]),
    /// Successful write/mkdir/unlink.
    Ok,
    /// Error code (see `types::ViError` discriminant).
    Err    (u8),
}

// ── Network service ───────────────────────────────────────────────────────────

/// Requests understood by the network service (`/bin/net`).
///
/// Opcodes mirror the existing VirtIO-backed syscalls but are typed.
#[derive(Debug, Serialize, Deserialize)]
pub enum NetRequest<'a> {
    Connect   { addr: [u8; 4], port: u16 },
    Send      { cap_id: u32, data: &'a [u8] },
    Recv      { cap_id: u32, buf_len: u32 },
    Close     { cap_id: u32 },
    Listen    { port: u16 },
    Accept    { cap_id: u32 },
    UdpCreate,
    UdpSend   { cap_id: u32, addr: [u8; 4], port: u16, data: &'a [u8] },
    UdpRecv   { cap_id: u32, buf_len: u32 },
    Resolve   { hostname: &'a str },
}

/// Responses from the network service.
#[derive(Debug, Serialize, Deserialize)]
pub enum NetResponse<'a> {
    CapId  (u32),
    Data   (&'a [u8]),
    Addr   ([u8; 4]),
    Ok,
    Err    (u8),
}

// ── Serialization helpers ─────────────────────────────────────────────────────

/// Serialize a typed message into `buf`.  Returns the used byte count.
///
/// # Errors
/// Returns an error if `buf` is too small for the encoded message.
pub fn encode<'a, T: Serialize>(msg: &T, buf: &'a mut [u8]) -> postcard::Result<&'a [u8]> {
    postcard::to_slice(msg, buf)
}

/// Deserialize a typed message from `buf`.
pub fn decode<'de, T: Deserialize<'de>>(buf: &'de [u8]) -> postcard::Result<T> {
    postcard::from_bytes(buf)
}
```

### Step 3 — Migrate VFS service (flag-day, user confirmed)

Since all Cell binaries are rebuilt from source (no committed binary blobs), a clean flag-day migration is used. VFS switches entirely to postcard; all callers (shell, net, tests) are updated in the same PR. No version-gate byte needed.

```rust
// cells/services/vfs/src/main.rs — replace entire opcode dispatch:
fn dispatch(buf: &[u8]) {
    match api::ipc::decode::<VfsRequest>(buf) {
        Ok(req) => handle_typed(req),
        Err(_) => {
            // Malformed message — log and send error response
            let _ = api::ipc::encode(&VfsResponse::Err(ViError::InvalidInput as u8), &mut resp_buf);
        }
    }
}
```

Update all callers (shell `cat`, `ls`, `echo`, net tests) to use `api::ipc::encode(&VfsRequest::GetFile { path }, &mut buf)` instead of raw opcode bytes.

---

## Todo List

- [ ] ⚠️ Confirm new `libs/api/src/ipc.rs` types with user (Law 1, 2x required)
- [ ] Add `postcard` + `serde` to `libs/api/Cargo.toml`
- [ ] Create `libs/api/src/ipc.rs` (VfsRequest, VfsResponse, NetRequest, NetResponse)
- [ ] Add `pub mod ipc;` to `libs/api/src/lib.rs`
- [ ] Update VFS service to decode VfsRequest (version-gate for backwards compat)
- [ ] `cargo check --workspace` — no errors
- [ ] Integration test: shell `cat /data/foo` round-trips through typed VfsRequest/VfsResponse

---

## Success Criteria

- [ ] `api::ipc::VfsRequest` visible from cell crates
- [ ] VFS service decodes `VfsRequest::GetFile` from a typed send
- [ ] Old VFS callers (using u8 opcodes) still work (version-gate)
- [ ] All 65 integration tests pass

---

## Risk Assessment

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| `postcard::MaxSize` not computable for `&str` fields | Medium | Document max path length (255 bytes); use manual `const` check in debug builds |
| Version-gate byte (0xFF) conflicts with existing opcode 0xFF | Low | Existing VFS opcodes are 0x01-0x0A (10 ops); 0xFF is safe |
| postcard serialization cost too high for high-frequency IPC | Low | ~20-50 cycles per encode; ecall overhead already >> 100 cycles |
