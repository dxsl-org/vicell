# Phase 01 — Wire Quota Enforcement to Write Path

**Status**: 📋 PLANNED  
**Priority**: P0  
**Effort**: 2 days

---

## Context Links

- Quota tracker: `cells/services/vfs/src/quota.rs` — `QuotaTracker`, `charge()`, `release()`
- VFS main loop: `cells/services/vfs/src/main.rs:521-528` — `VfsRequest::Write` handler
- Append handler: `cells/services/vfs/src/main.rs:529-540`
- VfsManager struct: `cells/services/vfs/src/main.rs:53-58` — has `quota: QuotaTracker` field
- Current limit: `cells/services/vfs/src/quota.rs:11` — `DEFAULT_QUOTA_BYTES = 32 * 1024 * 1024`

---

## Overview

`QuotaTracker` is fully implemented and compiles — it just isn't called anywhere in the write path. Both `Write` and `Append` handlers skip quota checking entirely. This phase wires the calls.

The VFS receives IPC requests that don't carry the CellId of the sender — but the VFS `sys_recv` call returns the sender's task ID. That task ID must be correlated to a CellId. Currently, VFS ignores the sender ID after receiving. We need to pass it to the quota system.

---

## Implementation Steps

### Step 1 — Thread sender ID through write dispatch

In `main.rs`, the `try_restore` loop variable `sender` is the task ID. Pass it as the "owner" to quota:

```rust
// Inside Write handler, before the actual FAT16/RamFS write:
let write_len = content.len() as u64;
// sender is the task_id of the calling Cell; use it as quota owner
let owner_cell_id = types::CellId(sender as u64);
if !gvfs.quota.can_charge(owner_cell_id, write_len) {
    // Return quota-exceeded error (reuse error byte 2 = QuotaExceeded)
    api::ipc::VfsResponse::Err(2)
} else {
    let ok = if path.starts_with("/data/") {
        write_fat16(fat_fs.as_ref(), path, content)
    } else if path.starts_with("/tmp/") {
        vfs.write_file(path, content)
    } else { false };
    if ok {
        gvfs.quota.charge(owner_cell_id, write_len);
        api::ipc::VfsResponse::Ok
    } else {
        api::ipc::VfsResponse::Err(1)
    }
}
```

### Step 2 — Add `can_charge()` method to `QuotaTracker`

The current `charge()` returns `ViResult<()>` and errors on overflow. Add a non-mutating check:

```rust
pub fn can_charge(&self, owner: CellId, bytes: u64) -> bool {
    let used = self.bytes_used.get(&owner.0).copied().unwrap_or(0);
    used + bytes <= self.limit_bytes(owner)
}

fn limit_bytes(&self, _owner: CellId) -> u64 {
    DEFAULT_QUOTA_BYTES
}
```

### Step 3 — Release quota on unlink

When a file is deleted via `Unlink`, charge back the file size:

```rust
// In Unlink handler, after successful delete:
// Look up file size before deletion (Stat), then release.
if let Some(stat) = vfs_stat(path) {
    gvfs.quota.release(owner_cell_id, stat.size);
}
```

### Step 4 — Wire Append similarly

Same pattern as Write but with `content.len()` delta.

---

## Todo List

- [ ] Add `can_charge(owner, bytes) -> bool` to `QuotaTracker`
- [ ] Wire quota check before Write handler writes to FAT16 or RamFS
- [ ] Wire quota release in Unlink handler
- [ ] Wire quota check before Append handler
- [ ] `cargo check -p service-vfs` — clean
- [ ] Test: write past 32MB limit → `VfsResponse::Err(2)` (quota exceeded)

---

## Success Criteria

- [ ] Writing 32MB + 1 byte as the same Cell returns `Err(2)` (quota exceeded)
- [ ] Deleting a file reduces the quota counter for that Cell
- [ ] Two different Cells each have their own quota (independent counters)
- [ ] All existing VFS integration tests pass
