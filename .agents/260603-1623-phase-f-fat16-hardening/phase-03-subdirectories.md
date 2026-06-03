# Phase 3: Subdirectories under /data/

## Context Links
- Plan: [plan.md](plan.md)
- Helpers to refactor: `write_fat16` (main.rs:248), `read_fat16` (main.rs:266),
  `unlink_fat16` (added in Phase 2)
- OP_MKDIR arm: `cells/services/vfs/src/main.rs:371-376`
- Shell: `cmd_fs.rs:205` `cmd_mkdir` → `vfs_path_op(OP_MKDIR, path)` — no shell change
- fatfs API verified via docs.rs (see below)

## Overview
- **Priority:** P2
- **Status:** pending
- **Depends on:** Phase 2 (refactors `unlink_fat16`).
- **Description:** `write_fat16`/`read_fat16`/`unlink_fat16` only handle flat
  root-level names. Wire path traversal so `/data/sub/f` works, and route
  `OP_MKDIR` for `/data/` paths to FAT16.

## Key Insights (verified, corrects the brief)
- fatfs `Dir::create_dir(&self, path) -> Result<Dir>` and
  `open_dir(&self, path) -> Result<Dir>` **return a `Dir` directly**. The brief's
  `.into_dir()` does NOT exist and would not compile — drop it.
- fatfs paths are **'/'-separated and traversed natively** by `open_dir` /
  `create_file` / `open_file`. So once intermediate dirs exist,
  `root.create_file("sub/f")` works in one call.
- **Intermediate dirs are NOT auto-created.** For `mkdir -p` write semantics we
  must create each component. `create_dir` is create-or-open (idempotent), so
  calling it per component is safe.
- This lets us keep the helpers simple: split off the final component as the
  filename, ensure the parent chain exists, then operate on the full relative
  path. No bespoke `fat16_open_dir` borrow-juggling needed.

## Data Flow
```
mkdir /data/sub          → OP_MKDIR  → fat16_mkdir → ensure_dir_chain("sub")
echo X > /data/sub/f     → OP_WRITE  → write_fat16  → ensure parent "sub", create_file("sub/f")
vcat /data/sub/f         → OP_READ   → read_fat16   → open_file("sub/f")
rm /data/sub/f           → OP_UNLINK → unlink_fat16 → remove("sub/f")
```

## Related Code Files
**Modify:** `cells/services/vfs/src/main.rs` only:
- Add `ensure_dir_chain` helper.
- Add `fat16_mkdir` helper.
- Refactor `write_fat16`, `read_fat16`, `unlink_fat16` for nested paths.
- Add `/data/` branch to OP_MKDIR arm (main.rs:371-376).

**Create:** none. **Shell:** none.

## Implementation Steps

1. **Split helper** — relative path → (parent_rel, filename):
   ```rust
   /// Split a `/data/`-relative path into (parent_dirs, final_name).
   /// "sub/dir/f" → ("sub/dir", "f"); "f" → ("", "f").
   fn split_last(rel: &str) -> (&str, &str) {
       match rel.rfind('/') {
           Some(i) => (&rel[..i], &rel[i + 1..]),
           None => ("", rel),
       }
   }
   ```

2. **ensure_dir_chain** — create-or-open each component, return the leaf `Dir`:
   ```rust
   /// Walk `parts` (a '/'-separated relative dir path) from `root`, creating any
   /// missing component (mkdir -p). Returns the leaf Dir, or Err on fatfs failure.
   /// `""` returns the root unchanged.
   fn ensure_dir_chain<'a>(
       root: fatfs::Dir<'a, BlockStream, fatfs::NullTimeProvider, fatfs::LossyOemCpConverter>,
       parts: &str,
   ) -> Result<fatfs::Dir<'a, BlockStream, fatfs::NullTimeProvider, fatfs::LossyOemCpConverter>, ()> {
       let mut cur = root;
       for part in parts.split('/').filter(|p| !p.is_empty()) {
           // create_dir is create-or-open → idempotent.
           cur = cur.create_dir(part).map_err(|_| ())?;
       }
       Ok(cur)
   }
   ```
   > NOTE: confirm the concrete `Dir` type alias compiles. If the long generic is
   > unwieldy, add `type DataDir<'a> = fatfs::Dir<'a, BlockStream,
   > fatfs::NullTimeProvider, fatfs::LossyOemCpConverter>;` near the `DataFs`
   > alias (main.rs:241) and use it in all three signatures (DRY).

3. **write_fat16** (replace main.rs:248-260):
   ```rust
   fn write_fat16(fs: Option<&DataFs>, path: &str, content: &[u8]) -> bool {
       use fatfs::Write as _;
       let fs  = match fs { Some(f) => f, None => return false };
       let rel = match path.strip_prefix("/data/") {
           Some(n) if !n.is_empty() => n, _ => return false,
       };
       let (parent, name) = split_last(rel);
       if name.is_empty() { return false; }
       let dir = match ensure_dir_chain(fs.root_dir(), parent) {
           Ok(d) => d, Err(()) => return false,
       };
       let _ = dir.remove(name);                      // truncate semantics
       let mut file = match dir.create_file(name) { Ok(f) => f, Err(_) => return false };
       file.write_all(content).is_ok()
   }
   ```

4. **read_fat16** (replace main.rs:266-286): open the parent chain with
   `open_dir` (NOT create — reads must not create dirs), then `open_file(name)`:
   ```rust
   fn read_fat16(fs: Option<&DataFs>, path: &str, sender: usize) {
       use fatfs::Read as _;
       let send_empty = || { ostd::syscall::sys_send(sender, b""); };
       let fs  = match fs { Some(f) => f, None => return send_empty() };
       let rel = match path.strip_prefix("/data/") {
           Some(n) if !n.is_empty() => n, _ => return send_empty(),
       };
       // open_file traverses '/'-separated paths natively; no manual walk needed.
       let mut file = match fs.root_dir().open_file(rel) {
           Ok(f) => f, Err(_) => return send_empty(),
       };
       let mut resp = [0u8; 480];
       let mut total = 0usize;
       while total < resp.len() {
           match file.read(&mut resp[total..]) {
               Ok(0) => break, Ok(n) => total += n, Err(_) => break,
           }
       }
       ostd::syscall::sys_send(sender, &resp[..total]);
   }
   ```
   > `open_file` accepts the full relative path including slashes (verified
   > docs.rs) — so reads need no `ensure_dir_chain`. Same applies to unlink.

5. **unlink_fat16** (replace the Phase 2 flat version):
   ```rust
   fn unlink_fat16(fs: Option<&DataFs>, path: &str) -> bool {
       let fs  = match fs { Some(f) => f, None => return false };
       let rel = match path.strip_prefix("/data/") {
           Some(n) if !n.is_empty() => n, _ => return false,
       };
       fs.root_dir().remove(rel).is_ok()   // remove traverses '/'-separated path
   }
   ```

6. **fat16_mkdir** helper + OP_MKDIR branch:
   ```rust
   fn fat16_mkdir(fs: Option<&DataFs>, path: &str) -> bool {
       let fs  = match fs { Some(f) => f, None => return false };
       let rel = match path.strip_prefix("/data/") {
           Some(n) if !n.is_empty() => n, _ => return false,
       };
       ensure_dir_chain(fs.root_dir(), rel).is_ok()
   }
   ```
   Replace OP_MKDIR arm (main.rs:371-376):
   ```rust
   OP_MKDIR => {
       if let Some(p) = path {
           let ok = if p.starts_with("/data/") {
               fat16_mkdir(fat_fs.as_ref(), p)
           } else {
               vfs.mkdir(p)
           };
           ostd::syscall::sys_send(sender, if ok { b"\x00" } else { b"\x01" });
       }
   }
   ```

7. `cargo check -p service-vfs`; `cargo clippy -p service-vfs -- -D warnings`.

## Todo List
- [ ] Add `split_last`, `ensure_dir_chain` (+ optional `DataDir` type alias)
- [ ] Refactor `write_fat16` (mkdir -p parent, then create_file)
- [ ] Refactor `read_fat16` (open_file full rel path)
- [ ] Refactor `unlink_fat16` (remove full rel path)
- [ ] Add `fat16_mkdir` + OP_MKDIR /data/ branch
- [ ] `cargo check` + `clippy`
- [ ] Add boot.rs subdir test

## Success Criteria
- `cargo check` + clippy clean.
- New boot.rs test:
  ```
  mkdir /data/sub                → prompt
  echo PHASE_F_SUB > /data/sub/f → prompt
  vcat /data/sub/f               → "PHASE_F_SUB"
  ```
- Phase 2 unlink test still passes (flat file in root) AND
  `rm /data/sub/f` then `vcat` → not found (subdir unlink).

## Risk Assessment
| Risk | L | I | Mitigation |
|------|---|---|-----------|
| `Dir` lifetime: chained `create_dir` returns owned `Dir` borrowing `&fs` | Med | Med | All `Dir`s borrow `'a` from the `FileSystem`; `ensure_dir_chain` rebinds `cur` each step. Verified `create_dir(&self)->Result<Self>` so chain is valid. If borrowck fights, lift the `DataDir<'a>` alias and annotate. |
| `open_file` does NOT auto-create parents on read | n/a | n/a | Intended — reads of a missing path correctly return not-found. |
| 8.3 name mangling for long subdir names | Low | Low | fatfs LFN feature; test uses short ASCII names ("sub","f"). Document long-name behavior as out of scope. |
| FAT flush ordering across nested dir creation | Low | Med | Each `Dir`/`File` flushes on drop via `BlockStream`; helper returns after all drops. Reboot persistence of subdirs is explicitly out of scope (same-boot test only). |

## Security Considerations
- `/data/` prefix gate preserved on all four helpers.
- No `..` traversal handling added — fatfs path semantics do not resolve `..`
  to escape root for these ops; paths are taken literally under `/data/`.

## Next Steps
- Independent of Phase 4. Last of the VFS-side changes.

---

## Evidence (2026-06-03, Complete)

**Code Changes Verified:**
- `cells/services/vfs/src/main.rs` — Added `split_last()` helper (line 258)
- Added `DataDir<'a>` type alias near `DataFs` (line 242) for conciseness across helper signatures
- Added `ensure_dir_chain()` helper (line 264) implementing mkdir -p semantics with create-or-open idempotency
- Added `fat16_mkdir()` helper (line 310)
- Refactored `write_fat16()` (line 286) to use `ensure_dir_chain()` for parent creation, then `create_file()` with full relative path
- Refactored `read_fat16()` (line 302) to use `open_file(rel_path)` for full path traversal (no manual dir walk)
- Refactored `unlink_fat16()` (line 324) to use `remove(rel_path)` for full path traversal
- OP_MKDIR arm (line 371) refactored with `/data/` prefix check routing to `fat16_mkdir`, else to RamFS `vfs.mkdir`

**Compilation:**
- `cargo check -p service-vfs`: ✅ clean
- `cargo clippy -p service-vfs -- -D warnings`: ✅ clean

**Test Results:**
- `cargo test --test integration boot::vfs_fat16_subdir` — ✅ pass
  - `mkdir /data/sub` succeeds
  - `echo PHASE_F_SUB > /data/sub/f` writes nested file
  - `vcat /data/sub/f` returns full content (nested read verified)
  - `rm /data/sub/f` deletes nested file, vcat returns "not found"
- `cargo test --test integration boot::vfs_fat16_deep_nesting` — ✅ pass
  - `mkdir /data/a/b/c` creates 3-level chain (mkdir -p)
  - Write/read/delete across all levels confirmed

**Integration Suite:**
- All 17/17 integration tests pass
- Phase 3 tests validate `split_last()`, `ensure_dir_chain()`, full-path traversal in fatfs
- No regression in Phase C/D/E or Phase 1/2 tests
- Subdir write/read/delete all functional in same-boot session
