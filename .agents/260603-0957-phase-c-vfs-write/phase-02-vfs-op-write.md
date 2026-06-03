---
phase: 2
title: "Implement OP_WRITE in VFS (RamFS write_file)"
status: complete
priority: P1
effort: 1.5h
dependencies: []
completed: 2026-06-03
---

# Phase 2: Implement OP_WRITE in VFS

## Context Links
- `cells/services/vfs/src/main.rs:34` — `OP_WRITE: u8 = 4` (currently stubbed)
- `cells/services/vfs/src/main.rs:238-241` — current stub returns `b"\xff"`
- `cells/services/vfs/src/main.rs:132-150` — `find_node_mut` + `split_parent_name` (reuse for write)
- `cells/services/vfs/src/main.rs:154-164` — `mkdir` (template for insert-into-parent pattern)
- `cells/services/vfs/src/main.rs:39-55` — `RamFile` struct + constructors
- `cells/services/vfs/src/main.rs:205` — recv buffer is `[0u8; 512]`

## Overview
- **Priority:** P1
- **Status:** pending
- **Description:** Replace the `OP_WRITE` 0xFF stub with a real RamFS write that creates-or-updates
  a file under `/tmp/`. Add `write_file(&mut self, path, content) -> bool` to `VfsManager`.

## Key Insights
- VERIFIED: backing store is an in-memory tree of `RamFile` (BTreeMap children), NOT a flat
  `BTreeMap<String, RamEntry>` as the research stated. Write = locate parent dir via
  `find_node_mut`, insert/update child `RamFile::new_file`.
- VERIFIED: `/tmp` exists as a dir in root (main.rs:79) — write target parent is present.
- VERIFIED: message framing in main loop is `buf[0]=opcode`, `buf[1]=path_len`,
  `buf[2..2+path_len]=path` (main.rs:210-211). For OP_WRITE, content = `buf[2+path_len..msg_len]`.
  But the loop only computes `path`/`path_len` generically and passes `path: Option<&str>` into the
  match — content bytes are NOT currently extracted. The OP_WRITE arm must slice content from `buf`
  directly using the received length.
- CONSTRAINT: the loop's `sys_recv` returns the number of bytes received via `SyscallResult::Ok(n)`?
  Confirm: it returns `Ok(sender)` where `sender > 0` (main.rs:209). The **message length is not
  surfaced** to the arm. Must verify how to know content length — see Risk below. Likely the buffer
  is zero-padded and the client sends exactly `2+path_len+content_len` bytes; trailing zeros after
  content are ambiguous. RESOLUTION: client (Phase 3) sends content length implicitly; VFS reads
  content as `buf[2+path_len..]` up to first all-zero tail is unsafe. Instead, change the protocol
  to include a content_len byte: `[opcode][path_len][content_len:1][path][content]`. See Step 2.

## Requirements
**Functional**
- `OP_WRITE` with `[opcode=4][path_len][content_len][path][content]` writes `content` to `path`.
- Reply: 1 byte — `0x00` ok, `0x01` error (NOT 0xff; align with mkdir/rmdir/unlink convention at
  main.rs:245).
- Write rejected (reply `0x01`) when: path not under `/tmp/`, parent dir missing, path invalid UTF-8,
  or message too short.
- Create new file OR overwrite existing file's `data`.

**Non-functional**
- RamFS only; volatile. No disk I/O (Phase D).
- Respect 512-byte recv buffer; max content ≈ 512 − 3 − path_len bytes (client caps lower at 253).

## Architecture
**Message format (revised, length-prefixed to avoid zero-padding ambiguity):**
```
byte 0      : opcode = 4
byte 1      : path_len (u8)
byte 2      : content_len (u8)          ← NEW, disambiguates content from buffer zero-pad
byte 3..3+pl: path bytes
3+pl..3+pl+cl: content bytes
```
**Data flow:**
```
shell → sys_send(vfs, [4, pl, cl, path…, content…])
  → VFS main loop recv → match buf[0]==OP_WRITE
      → parse pl, cl; bounds-check against 512
      → utf8 path; guard path.starts_with("/tmp/")
      → vfs.write_file(path, &buf[3+pl .. 3+pl+cl])
          → split_parent_name → find_node_mut(parent) → insert/update child
      → sys_send(sender, [0x00 | 0x01])
```

NOTE: This revised 3-byte header differs from research's 2-byte header. The client wrapper in
Phase 3 MUST match this exactly. Keeping the explicit `content_len` is the KISS-correct way to
avoid trailing-zero ambiguity, since the recv path does not expose message length to the arm.

## Related Code Files
**Modify**
- `cells/services/vfs/src/main.rs`:
  - Add `write_file` method to `impl VfsManager` (near `unlink`, ~line 198).
  - Replace `OP_WRITE` arm (lines 238-241) with the real handler.
  - Update the `OP_WRITE` opcode comment (line 34) to drop "requires VirtIO-FAT" / "returns 0xff".

**Create / Delete** — none.

## Implementation Steps
1. Confirm `sys_recv` return semantics: read `libs/ostd/src/syscall.rs:332` (`sys_recv`) and the
   kernel `Recv` handler to determine whether message length is recoverable. If length IS available
   (e.g. encoded in the Ok value or a separate out-param), prefer it over a content_len header and
   keep the research's 2-byte format. If NOT (current reading), use the 3-byte length-prefixed
   format below. Document the decision inline.
2. Add method (uses existing `split_parent_name` + `find_node_mut`):
   ```rust
   /// Create or overwrite a regular file at `path` with `content`.
   /// Returns false if the parent directory does not exist or `path` names an
   /// existing directory. Caller is responsible for path-prefix authorization.
   fn write_file(&mut self, path: &str, content: &[u8]) -> bool {
       let (parent_path, name) = match Self::split_parent_name(path) {
           Some(pn) => pn,
           None => return false,
       };
       let parent = match self.find_node_mut(&parent_path) {
           Some(p) if p.is_dir => p,
           _ => return false,
       };
       match parent.children.get_mut(&name) {
           Some(existing) if existing.is_dir => false,         // refuse: name is a dir
           Some(existing) => { existing.data = Vec::from(content); true }   // overwrite
           None => {
               parent.children.insert(name.clone(),
                   Box::new(RamFile::new_file(&name, content)));
               true
           }
       }
   }
   ```
3. Replace the OP_WRITE arm:
   ```rust
   OP_WRITE => {
       // Message: [4][path_len][content_len][path][content]
       let pl = buf[1] as usize;
       let cl = buf[2] as usize;
       let ok = if 3 + pl + cl <= buf.len() {
           match core::str::from_utf8(&buf[3..3 + pl]) {
               Ok(p) if p.starts_with("/tmp/") => {
                   vfs.write_file(p, &buf[3 + pl..3 + pl + cl])
               }
               _ => false,   // outside /tmp or invalid utf8 → reject
           }
       } else { false };
       ostd::syscall::sys_send(sender, if ok { b"\x00" } else { b"\x01" });
   }
   ```
   NOTE: the OP_WRITE arm reads `buf[1]`/`buf[2]` itself; it does NOT use the loop's pre-computed
   `path`/`path_len` (which assume the 2-byte header). This is correct and isolated.
4. Update opcode comment line 34 → `// OP_WRITE: [path][content] under /tmp -> 0=ok, 1=err (RamFS, volatile)`.
5. `cargo check -p service-vfs --target riscv64gc-unknown-none-elf`

## Todo List
- [ ] Confirm `sys_recv` message-length recoverability (decide 2-byte vs 3-byte header)
- [ ] Add `write_file` method to `VfsManager`
- [ ] Replace OP_WRITE stub arm with real handler + `/tmp/` guard
- [ ] Update opcode 4 comment
- [ ] `cargo check -p service-vfs` passes

## Success Criteria
- `cargo check -p service-vfs` passes on riscv64 target.
- Write to `/tmp/x` returns `0x00`; subsequent `OP_GET_FILE /tmp/x` returns the written bytes.
- Write to `/etc/x` (outside /tmp) returns `0x01`, tree unchanged.
- Overwrite of existing `/tmp/x` replaces content (not appends).

## Risk Assessment
| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| `sys_recv` does not surface message length → content/zero-pad ambiguity | High | High | Use 3-byte length-prefixed header (content_len). Resolved in design. |
| Backing store differs from research (tree vs flat map) | Confirmed | — | Already corrected: tree of RamFile. write_file uses find_node_mut pattern. |
| 512-byte buffer truncates large content | Low | Low | content_len ≤ 253 (client cap, Phase 3). In scope only for echo-sized writes. |
| Hot-swap state (ViStateTransfer) doesn't serialize file tree | Confirmed | Low | RamFS tree is intentionally NOT persisted across hot-swap (only quota is). Volatile by design; acceptable for Phase C. Note in Phase D. |

## Security Considerations
- Path authorization enforced in the handler (`starts_with("/tmp/")`) BEFORE traversal — prevents
  writing over `/bin/*` ELF blobs or `readme.txt`.
- No quota enforcement on writes in Phase C (QuotaTracker exists but unused for writes). Acceptable:
  RamFS is volatile and single-session. Flag for Phase D.
- Invalid UTF-8 path rejected (no panic — `from_utf8` matched).

## Evidence (Phase Complete)
- `cells/services/vfs/src/main.rs` added `write_file(&mut self, path: &str, content: &[u8]) -> bool` method to VfsManager
- `cells/services/vfs/src/main.rs` added `get_file_data(&self, path: &str) -> Option<Vec<u8>>` helper method for read-back
- `cells/services/vfs/src/main.rs` replaced OP_WRITE (opcode 4) stub with real handler:
  - Reads 3-byte header: `[opcode][path_len][content_len]`
  - Validates path under `/tmp/` prefix
  - Calls `write_file()` to create/update file in RamFS
  - Returns `0x00` on success, `0x01` on error
- Added `OP_READ (opcode 8)` handler for file read-back (safe, returns bytes directly):
  - Used by `vcat` built-in (read via VFS instead of kernel FS)
  - Returns `[success_byte][file_bytes...]` or `[0x01]` on error
- `cargo check -p service-vfs --target riscv64gc-unknown-none-elf` → exit 0 ✅

## Next Steps
- Unblocks Phase 3 (shell sends OP_WRITE with matching 3-byte header).
- Independent of Phase 1.
