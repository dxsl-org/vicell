# Phase 03 — Capability-Based Path Access Control

**Status**: 📋 PLANNED  
**Priority**: P1  
**Effort**: 4 days

---

## Context Links

- Current path auth: `cells/services/vfs/src/main.rs:522-526` — hardcoded `/data/` and `/tmp/` prefix checks
- VFS main.rs sender: `sys_recv` returns `sender: usize` (task ID of calling cell)
- Cap registry: `kernel/src/cell/cap_registry.rs` (if it exists) — cell capability table
- ZST caps: `kernel/src/task/cap.rs` — `BlockIoCap`, `NetworkCap`, `SpawnCap` (existing)
- Mount table: `cells/services/vfs/src/mount.rs` — `MountTable`, `MountEntry { prefix, writable }`

---

## Design: CellId-Based Path Rules (Not POSIX)

**Research finding**: POSIX uid/gid/mode bits are inappropriate for a SAS OS with ZST capability tokens. Capability-based model:

- Each VFS path prefix has a set of allowed CellIds (or "all cells") per operation
- VFS checks sender CellId against the rule table before executing any write
- Rules are configured at VFS startup (hardcoded for MVP; future: loaded from manifest)

**Rule model:**
```
PathRule {
    prefix: &str,          // "/data/"
    allow_read: CellSet,   // who can read
    allow_write: CellSet,  // who can write
}

enum CellSet {
    All,                   // any cell (current behavior for read)
    Specific(Vec<CellId>), // only listed cells
    None,                  // nobody
}
```

For Phase 03 MVP: hardcode rules at VFS startup. Future Phase 30 will load them from ELF `.ViCell_manifest`.

---

## Implementation Steps

### Step 1 — Add `AccessTable` to VFS

```rust
// cells/services/vfs/src/access.rs (new file)
use types::CellId;
use alloc::vec::Vec;

pub struct PathRule {
    pub prefix: alloc::string::String,
    pub allow_read_all: bool,     // true = any cell may read
    pub allow_write: Vec<u64>,    // cell_id.0 values that may write; empty = none
}

pub struct AccessTable {
    rules: Vec<PathRule>,
}

impl AccessTable {
    pub fn default_rules() -> Self {
        Self {
            rules: alloc::vec![
                // /data/ — readable by all, writable by all (Phase 03: open for now)
                PathRule { prefix: "/data/".into(), allow_read_all: true, allow_write: alloc::vec![] },
                // /tmp/ — readable and writable by all (volatile scratch)
                PathRule { prefix: "/tmp/".into(), allow_read_all: true, allow_write: alloc::vec![] },
                // /bin/ — read-only for everyone
                PathRule { prefix: "/bin/".into(), allow_read_all: true, allow_write: alloc::vec![] },
            ],
        }
    }

    /// Check if `cell_id` may write to `path`.
    pub fn can_write(&self, cell_id: CellId, path: &str) -> bool {
        for rule in &self.rules {
            if path.starts_with(rule.prefix.as_str()) {
                // Empty allow_write = all cells may write (open policy)
                return rule.allow_write.is_empty()
                    || rule.allow_write.contains(&cell_id.0);
            }
        }
        false // no matching rule → deny
    }

    /// Check if `cell_id` may read from `path`.
    pub fn can_read(&self, cell_id: CellId, path: &str) -> bool {
        let _ = cell_id;
        for rule in &self.rules {
            if path.starts_with(rule.prefix.as_str()) {
                return rule.allow_read_all;
            }
        }
        false
    }
}
```

### Step 2 — Add `access: AccessTable` to `VfsManager`

```rust
pub struct VfsManager {
    root:    Box<RamFile>,
    handles: HandleTable,
    mounts:  MountTable,
    quota:   QuotaTracker,
    access:  AccessTable,       // NEW
}
```

Initialize with `AccessTable::default_rules()` in `VfsManager::new()`.

### Step 3 — Gate Write/Append/Mkdir/Rmdir/Unlink operations

At the top of each mutating IPC handler (before the actual operation):

```rust
let sender_cell = types::CellId(sender as u64);
if !gvfs.access.can_write(sender_cell, path) {
    return api::ipc::VfsResponse::Err(3); // 3 = PermissionDenied
}
```

Gate GetFile / ListDir / Stat behind `can_read()` check.

### Step 4 — Future extension hooks (doc only, not implement)

Document where Phase 30 (ELF manifests) will inject rules:
- `loader.rs::spawn_from_path()` reads `.ViCell_manifest` ELF section
- Extracts `vfs_write_paths: ["/data/apps/myapp/"]` capability
- Calls `vfs.access.add_rule(cell_id, "/data/apps/myapp/", write=true)`

For Phase 03, all cells can write to `/data/` (unchanged from current). The value is: the enforcement infrastructure is in place for Phase 30 to plug in.

---

## Todo List

- [ ] Create `cells/services/vfs/src/access.rs` with `AccessTable` + `PathRule`
- [ ] Add `access: AccessTable` to `VfsManager`
- [ ] Gate Write, Append, Mkdir, Rmdir, Unlink behind `can_write(sender_cell, path)`
- [ ] Gate GetFile, ListDir, Stat behind `can_read(sender_cell, path)` (all-allow for now)
- [ ] Add `pub mod access;` to VFS lib
- [ ] `cargo check -p service-vfs` — clean
- [ ] Document Phase 30 extension points in `access.rs` comments

---

## Success Criteria

- [ ] `AccessTable::can_write` returns false for unknown path prefixes
- [ ] Write to `/bin/` (read-only prefix) returns `Err(3)` PermissionDenied
- [ ] Write to `/data/` still works for all cells (open policy in Phase 03)
- [ ] All existing VFS integration tests pass (no behavior change for `/data/`)
