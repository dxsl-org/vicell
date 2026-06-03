# Phase 1: OP_WRITE Header Widening (4-byte header)

## Context Links
- Plan: [plan.md](plan.md)
- Client: `cells/apps/shell/src/cmd_fs.rs:256-279` (`write_file`)
- Caller: `cells/apps/shell/src/executor.rs:93` (echo-redirect)
- Server: `cells/services/vfs/src/main.rs:340-358` (OP_WRITE arm)
- Test pattern: `tests/integration/tests/boot.rs:303` (Phase C)

## Overview
- **Priority:** P2
- **Status:** pending
- **Description:** `content_len` is 1 byte today, capping echo redirect at
  `253 - path_len` content bytes. Widen to a 2-byte LE `u16` → up to 65535
  content bytes (still bounded by the 512-byte client buffer, so effective cap
  rises to `512 - 4 - pl`).

## Key Insights
- Server `buf` is already 512 bytes (main.rs:292); only the header parse changes.
- Existing tests assert observable behavior (write then `vcat` returns the
  marker), not wire bytes — so widening is non-breaking for them.
- The shell writer is `write_file` (NOT `vfs_write_echo_redirect` — that name
  does not exist; brief was wrong). It is the only OP_WRITE producer.
- `vfs_path_op` (cmd_fs.rs:45) is a SEPARATE 2-byte header for MKDIR/RMDIR/UNLINK
  and is untouched.

## Data Flow
```
echo X > /data/f
  └ executor.rs:93  cmd_fs::write_file("/data/f", b"X\n")
       └ build 4-byte header buf, sys_send(VFS_ENDPOINT=3, ..)
            └ vfs main loop sys_recv → OP_WRITE arm
                 └ parse pl=buf[1], cl=u16(buf[2],buf[3])
                      └ /data/ → write_fat16 ; /tmp/ → vfs.write_file
                           └ reply 0x00 ok / 0x01 err
```

## New Wire Format
```
byte 0   : opcode = 4 (OP_WRITE)
byte 1   : path_len  (u8)
bytes 2-3: content_len (u16 LE)   ← was 1 byte at byte[2]
bytes 4..4+pl       : path
bytes 4+pl..4+pl+cl : content
```

## Related Code Files
**Modify:**
- `cells/apps/shell/src/cmd_fs.rs` — `write_file` only (and its doc comment at :259).
- `cells/services/vfs/src/main.rs` — OP_WRITE arm only; update doc comment at :35.

**Create:** none.

## Implementation Steps

1. **Client (`cmd_fs.rs:263-279`)** — replace `write_file` body:
   ```rust
   /// Write `content` to `path` via VFS OP_WRITE (4-byte header:
   /// opcode, path_len:u8, content_len:u16 LE). Path+content capped to the
   /// 512-byte client buffer. The VFS server enforces /data//tmp authorization.
   pub fn write_file(path: &str, content: &[u8]) -> bool {
       let pb = path.as_bytes();
       let pl = pb.len().min(255);                       // path_len fits u8
       let cl = content.len().min(512_usize.saturating_sub(4 + pl));
       let mut buf = [0u8; 512];
       buf[0] = OP_WRITE;
       buf[1] = pl as u8;
       buf[2..4].copy_from_slice(&(cl as u16).to_le_bytes());
       buf[4..4 + pl].copy_from_slice(&pb[..pl]);
       buf[4 + pl..4 + pl + cl].copy_from_slice(&content[..cl]);
       syscall::sys_send(VFS_ENDPOINT, &buf[..4 + pl + cl]);
       let mut reply = [0u8; 1];
       match syscall::sys_recv(0, &mut reply) {
           syscall::SyscallResult::Ok(_) => reply[0] == 0,
           _ => false,
       }
   }
   ```

2. **Server (`main.rs:340-358`)** — replace the OP_WRITE arm:
   ```rust
   OP_WRITE => {
       // Header: [4][path_len:u8][content_len:u16 LE][path][content]
       let pl = buf[1] as usize;
       let cl = u16::from_le_bytes([buf[2], buf[3]]) as usize;
       let ok = if 4 + pl + cl <= buf.len() {
           match core::str::from_utf8(&buf[4..4 + pl]) {
               Ok(p) if p.starts_with("/data/") =>
                   write_fat16(fat_fs.as_ref(), p, &buf[4 + pl..4 + pl + cl]),
               Ok(p) if p.starts_with("/tmp/") =>
                   vfs.write_file(p, &buf[4 + pl..4 + pl + cl]),
               _ => false,
           }
       } else { false };
       ostd::syscall::sys_send(sender, if ok { b"\x00" } else { b"\x01" });
   }
   ```

3. Update the `OP_WRITE` doc comment at `main.rs:35` to say `[path_len:u8][content_len:u16 LE]`.

4. `cargo check -p app-shell -p service-vfs`.

## Todo List
- [ ] Rewrite `cmd_fs.rs::write_file` (4-byte header, 512-byte buf, `255`/`512-4-pl` caps)
- [ ] Rewrite `main.rs` OP_WRITE arm (u16 LE parse, offset 4)
- [ ] Update both doc comments
- [ ] `cargo check` both crates
- [ ] Re-run Phase C/D/E tests (no regression)
- [ ] Add new boot.rs test for a >253-byte write

## Success Criteria
- `cargo check` clean on both crates.
- Existing `vfs_write_echo_redirect`, `vfs_fat16_write_read`,
  `vfs_fat16_reboot_persistence` still pass.
- New test: write a marker line longer than 253 bytes to `/tmp/big.txt`, `vcat`
  returns the full content (proves the cap rose above 253).

## Risk Assessment
| Risk | L | I | Mitigation |
|------|---|---|-----------|
| Mismatched header length client vs server | Low | High | Both changed in one phase; offsets cross-checked (both use 4). |
| `pl` truncation if path >255 | Low | Low | `.min(255)` clamps; paths are short. |

## Security Considerations
- Server still bounds `4 + pl + cl <= buf.len()` (512) before slicing — no OOB read.
- `/data//tmp` prefix authorization unchanged; non-allowlisted paths still rejected.

## Next Steps
- Independent of Phases 2/4. Precedes Phase 3 (same file, disjoint region).

---

## Evidence (2026-06-03, Complete)

**Code Changes Verified:**
- `cells/apps/shell/src/cmd_fs.rs:263-279` — `write_file()` rewritten with 4-byte header `[OP_WRITE][path_len:u8][content_len:u16 LE][path][content]`, 512-byte buffer
- `cells/services/vfs/src/main.rs:340-358` — OP_WRITE arm updated to parse `u16::from_le_bytes([buf[2], buf[3]])` for content length, offset 4 for path
- Both doc comments updated

**Compilation:**
- `cargo check -p app-shell`: ✅ clean
- `cargo check -p service-vfs`: ✅ clean

**Test Results:**
- `cargo test --test integration boot::vfs_write_echo_redirect` — ✅ pass (Phase C test, no regression)
- `cargo test --test integration boot::vfs_fat16_write_read` — ✅ pass (Phase D test)
- `cargo test --test integration boot::vfs_fat16_reboot_persistence` — ✅ pass (Phase E test)
- `cargo test --test integration boot::vfs_fat16_large_write` — ✅ pass (>253-byte write to /tmp/big.txt via 4-byte header, vcat verifies full content read-back)

**Integration Suite:**
- All 17/17 integration tests pass in single QEMU boot
- `vfs_fat16_large_write` directly validates that the 4-byte header widening raised the cap above 253 bytes
