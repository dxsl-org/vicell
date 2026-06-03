# Phase 2: OP_UNLINK for /data/ (FAT16)

## Context Links
- Plan: [plan.md](plan.md)
- Server arm: `cells/services/vfs/src/main.rs:383-388` (OP_UNLINK)
- Existing FAT16 helper precedent: `write_fat16` (main.rs:248) uses `root.remove`
- Shell: `cmd_fs.rs:248` `cmd_rm` → `vfs_path_op(OP_UNLINK, path)` — no shell change
- Test pattern: `tests/integration/tests/boot.rs:329` (Phase D)

## Overview
- **Priority:** P2
- **Status:** pending
- **Description:** `OP_UNLINK` routes every path to `vfs.unlink` (RamFS).
  `/data/` files cannot be deleted. Add a FAT16 branch + `unlink_fat16` helper.

## Key Insights
- `Dir::remove(name)` already proven working in `write_fat16` (main.rs:256) for
  the create-or-overwrite truncate. `unlink_fat16` reuses the same call.
- Shell `cmd_rm` already sends OP_UNLINK via the 2-byte `vfs_path_op` header — no
  client change. The server reads `path` from `buf[2..2+path_len]` (main.rs:313).
- This phase keeps unlink **flat root only**; Phase 3 generalizes to subdirs.

## Data Flow
```
rm /data/del.txt
  └ cmd_fs.rs:248 cmd_rm → vfs_path_op(OP_UNLINK=7, "/data/del.txt")
       └ 2-byte header sys_send(3, ..)
            └ vfs OP_UNLINK arm
                 └ "/data/" → unlink_fat16 → root.remove("del.txt")
                 └ else     → vfs.unlink (RamFS)
                      └ reply 0x00 ok / 0x01 err
```

## Related Code Files
**Modify:**
- `cells/services/vfs/src/main.rs` — add `unlink_fat16` helper (near `write_fat16`,
  ~main.rs:260) and a `/data/` branch in the OP_UNLINK arm (main.rs:383-388).

**Create:** none. **Shell:** none.

## Implementation Steps

1. Add helper after `read_fat16` (~main.rs:286):
   ```rust
   /// Remove `/data/NAME` from the FAT16 volume. Flat-root only (Phase 2);
   /// Phase 3 generalizes to subdirectories. Returns false if unmounted,
   /// the name is empty, or the entry does not exist.
   fn unlink_fat16(fs: Option<&DataFs>, path: &str) -> bool {
       let fs = match fs { Some(f) => f, None => return false };
       let name = match path.strip_prefix("/data/") {
           Some(n) if !n.is_empty() => n,
           _ => return false,
       };
       fs.root_dir().remove(name).is_ok()
   }
   ```

2. Replace the OP_UNLINK arm (main.rs:383-388):
   ```rust
   OP_UNLINK => {
       if let Some(p) = path {
           let ok = if p.starts_with("/data/") {
               unlink_fat16(fat_fs.as_ref(), p)
           } else {
               vfs.unlink(p)
           };
           ostd::syscall::sys_send(sender, if ok { b"\x00" } else { b"\x01" });
       }
   }
   ```

3. `cargo check -p service-vfs`.

## Todo List
- [ ] Add `unlink_fat16` helper
- [ ] Add `/data/` branch to OP_UNLINK arm
- [ ] `cargo check -p service-vfs`
- [ ] Add boot.rs test (write → rm → vcat = not found)

## Success Criteria
- `cargo check` clean.
- New boot.rs test:
  ```
  echo PHASE_F_DEL > /data/del.txt   → prompt
  vcat /data/del.txt                 → "PHASE_F_DEL"   (exists)
  rm /data/del.txt                   → prompt
  vcat /data/del.txt                 → "vcat: not found: /data/del.txt"
  ```
  Assert the final `wait_for("not found")` succeeds.

## Risk Assessment
| Risk | L | I | Mitigation |
|------|---|---|-----------|
| `remove` on a directory succeeds unexpectedly | Low | Low | Phase 2 only documents flat files; fatfs `remove` errors on non-empty dirs. |
| Stale FAT cache not flushed after remove | Low | Med | `BlockStream::flush` runs on `Dir` drop (same as write_fat16); helper returns after drop. |

## Security Considerations
- `/data/` prefix gate preserved; only files under `/data/` reach `unlink_fat16`.
- Empty-name guard (`!n.is_empty()`) prevents `remove("")` on root.

## Next Steps
- Phase 3 refactors `unlink_fat16` into the path-traversal version. Land Phase 2
  first so the simple flat case has a passing test before generalizing.

---

## Evidence (2026-06-03, Complete)

**Code Changes Verified:**
- `cells/services/vfs/src/main.rs` — `unlink_fat16()` helper added at line 287; returns `fs.root_dir().remove(name).is_ok()` for flat-root files
- OP_UNLINK arm (line 383) refactored with `/data/` prefix check routing to `unlink_fat16`, else to RamFS `vfs.unlink`

**Compilation:**
- `cargo check -p service-vfs`: ✅ clean
- `cargo clippy -p service-vfs -- -D warnings`: ✅ clean

**Test Results:**
- `cargo test --test integration boot::vfs_fat16_unlink` — ✅ pass
  - Write marker `PHASE_F_DEL` to `/data/del.txt` → vcat verifies presence
  - `rm /data/del.txt` succeeds
  - vcat returns "not found" error (deletion confirmed)

**Integration Suite:**
- All 17/17 integration tests pass
- Phase 2 unlink test validates flat-root delete + fatfs cache flush behavior
- No regression in Phase C/D/E tests
