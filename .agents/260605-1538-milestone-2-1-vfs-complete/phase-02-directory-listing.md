# Phase 02 — Complete Directory Operations

**Status**: 📋 PLANNED  
**Priority**: P1  
**Effort**: 3 days

---

## Context Links

- `VfsManager::list_dir()`: `cells/services/vfs/src/main.rs:99-113` — only lists RamFS root children
- `ListDir` IPC handler: `cells/services/vfs/src/main.rs:502-510` — 480-byte limit, no FAT16 recursion
- FAT16 dir access: `cells/services/vfs/src/main.rs:249-418` — uses `fatfs::Dir::iter()`
- RamFS `find_node()`: `cells/services/vfs/src/main.rs:84-91` — path traversal helper
- `VfsResponse::Data`: returns newline-separated names (all names, no size/type metadata)

---

## Overview

`ListDir` currently works but has two gaps:

1. **FAT16 subdirectory listing**: `ListDir("/data/subdir")` returns empty because `list_dir()` only traverses the RamFS tree, not FAT16. FAT16 listing goes through `fatfs::Dir::iter()` which exists but isn't called from `ListDir`.

2. **No type metadata**: Callers can't distinguish files from directories without a separate `Stat` call per entry. Add an optional type prefix: `d:name\n` for dirs, `f:name\n` for files (backwards-compatible: callers ignoring the prefix still get names).

---

## Implementation Steps

### Step 1 — FAT16 subdirectory listing

Add a `list_fat16_dir(path: &str) -> alloc::string::String` function:

```rust
fn list_fat16_dir(fs: Option<&DataFs>, path: &str) -> alloc::string::String {
    let Some(fs) = fs else { return String::new() };
    let root = fs.root_dir();
    let dir = match root.open_dir(path.trim_start_matches("/data/")) {
        Ok(d) => d,
        Err(_) => return String::new(),
    };
    let mut result = String::new();
    for entry in dir.iter() {
        if let Ok(e) = entry {
            let name = e.file_name();
            if name == "." || name == ".." { continue; }
            if e.is_dir() {
                result.push_str("d:");
            } else {
                result.push_str("f:");
            }
            result.push_str(&name);
            result.push('\n');
        }
    }
    result
}
```

Update `ListDir` IPC handler to route `/data/` paths through `list_fat16_dir`.

### Step 2 — RamFS type-prefixed listing

Update `VfsManager::list_dir()` to emit `d:name\n` or `f:name\n`:

```rust
fn list_dir(&self, path: &str, out: &mut [u8]) -> usize {
    let node = match self.find_node(path) {
        Some(n) if n.is_dir => n,
        _ => return 0,
    };
    let mut pos = 0;
    for (name, child) in node.children.iter() {
        let prefix = if child.is_dir { "d:" } else { "f:" };
        let entry = alloc::format!("{}{}\n", prefix, name);
        let b = entry.as_bytes();
        if pos + b.len() > out.len() { break; }
        out[pos..pos + b.len()].copy_from_slice(b);
        pos += b.len();
    }
    pos
}
```

### Step 3 — Remove the 480-byte hardcoded limit

The current `ListDir` response is encoded into a 480-byte temporary buffer. Switch to using the full 512-byte IPC buffer and postcard encoding for `VfsResponse::Data(&[u8])`.

### Step 4 — Add `VfsRequest::ListDirV2` (backwards-compat)

To avoid breaking existing callers that don't expect type prefixes, add a new `ListDirV2` variant that returns the prefixed format. Keep old `ListDir` unchanged.

OR: Since both shell and tests are in-tree and can be updated atomically, do a flag-day migration (simpler). Update all callers to parse `d:` / `f:` prefix, stripping prefix when only the name is needed.

---

## Todo List

- [ ] Add `list_fat16_dir(fs, path)` function routing FAT16 subdirectory listing
- [ ] Update `ListDir` handler to route `/data/subdir` through `list_fat16_dir`
- [ ] Update `VfsManager::list_dir()` to emit `d:`/`f:` type prefixes
- [ ] Update shell `ls` command to strip `d:`/`f:` prefix when displaying
- [ ] `cargo check -p service-vfs -p app-shell` — clean
- [ ] Test: `ls /data/` shows entries; `ls /data/subdir` shows subdir entries

---

## Success Criteria

- [ ] `ListDir("/data/")` returns all files and directories in FAT16 root
- [ ] `ListDir("/data/subdir")` returns contents of a FAT16 subdirectory
- [ ] `ListDir("/tmp")` returns RamFS volatile entries
- [ ] Type prefix (`d:`/`f:`) allows callers to distinguish dirs from files
- [ ] `ls` shell command works correctly for both RamFS and FAT16 paths
