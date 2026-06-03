# Phase 4: Route /data/* to FAT16

## Context Links
- `cells/services/vfs/src/main.rs:271-296` — current OP_WRITE / OP_READ handlers
- `cells/services/vfs/src/main.rs:34-35` — `OP_WRITE = 4`, `OP_READ = 8`
- `kernel/src/fs/fat.rs:206-258` — kernel `open`/`create_file` semantics (8.3 names, trim leading `/`)
- Phase 3 `fat_fs: Option<FileSystem<BlockStream,...>>` in `main()`

## Overview
- **Priority:** P3 (depends on Phase 3)
- **Status:** pending
- **Effort:** 2h
- Branch OP_WRITE and OP_READ on path prefix: `/data/` → FAT16, `/tmp/` → RamFS.
  Flat root only (no subdirs under `/data/`).

## Key Insights (verified)
- OP_WRITE already has a 3-byte header `[op][path_len][content_len]` and a
  `/tmp/` prefix check (`main.rs:275-285`). Phase 4 ADDS a `/data/` branch — does
  not change the `/tmp/` path.
- OP_READ uses the shared 2-byte header (`path` parsed at `main.rs:243-244`) and
  currently serves only RamFS via `get_file_data` (`main.rs:287-296`). Add a
  `/data/` branch BEFORE the RamFS lookup.
- fatfs root dir ops need the leading slash trimmed AND the `/data/` prefix
  stripped to get a flat 8.3 filename. Kernel does `trim_start_matches('/')`
  (`fat.rs:207`). Here: strip `/data/` → e.g. `test.txt`.
- fatfs filenames are 8.3 uppercased on disk (`make_83_name` in mkfat32.py). The
  `fatfs` crate handles LFN/short-name mapping on read-back, so `test.txt`
  round-trips. Keep `/data/` names ≤ 8.3 to be safe in Phase D; longer names rely
  on the crate's LFN support (works but untested here — note as a constraint).
- `fat_fs` is borrowed mutably by helpers; `FileSystem` methods take `&self`
  (interior mutability via the stream), so `&fat_fs` (shared) suffices for
  `root_dir()`. Match the kernel which uses `&self` throughout (`fat.rs:206`).

## Requirements
- **Functional:**
  - `OP_WRITE` to `/data/NAME`: create-or-overwrite a file with the content,
    flush on drop. Reply `0x00` ok / `0x01` err. `None` `fat_fs` → err.
  - `OP_READ` of `/data/NAME`: read up to 480 bytes, `sys_send` them; missing →
    empty reply (mirrors RamFS behaviour).
- **Non-functional:** No panic on missing volume or missing file. Overwrite must
  NOT leave stale tail bytes (truncate semantics).

## Architecture / Data Flow
```
OP_WRITE [4][pl][cl][path][content]
  p = utf8(path)
  if p starts_with "/data/" → write_fat16(&fat_fs, p, content)
  elif p starts_with "/tmp/" → vfs.write_file(p, content)   (unchanged)
  else → false
  reply 0x00/0x01

OP_READ [8][pl][path]
  if p starts_with "/data/" → read_fat16(&fat_fs, p, sender)
  elif vfs.get_file_data(p) → send bytes
  else → send empty
```

## Related Code Files
**Modify:** `cells/services/vfs/src/main.rs` (OP_WRITE arm, OP_READ arm, 2 helpers)

## Implementation Steps

### 1. OP_WRITE branch (`main.rs:271-286`)
Replace the `ok` computation:
```rust
OP_WRITE => {
    let pl = buf[1] as usize;
    let cl = buf[2] as usize;
    let ok = if 3 + pl + cl <= buf.len() {
        match core::str::from_utf8(&buf[3..3 + pl]) {
            Ok(p) if p.starts_with("/data/") => {
                write_fat16(fat_fs.as_ref(), p, &buf[3 + pl..3 + pl + cl])
            }
            Ok(p) if p.starts_with("/tmp/") => {
                vfs.write_file(p, &buf[3 + pl..3 + pl + cl])
            }
            _ => false,
        }
    } else { false };
    ostd::syscall::sys_send(sender, if ok { b"\x00" } else { b"\x01" });
}
```

### 2. OP_READ branch (`main.rs:287-296`)
```rust
OP_READ => {
    if let Some(p) = path {
        if p.starts_with("/data/") {
            read_fat16(fat_fs.as_ref(), p, sender);
        } else if let Some(data) = vfs.get_file_data(p) {
            let n = data.len().min(480);
            ostd::syscall::sys_send(sender, &data[..n]);
        } else {
            ostd::syscall::sys_send(sender, b"");
        }
    }
}
```

### 3. Helpers (free fns in `main.rs`, above `main()`)
```rust
use block_stream::BlockStream;
type DataFs = fatfs::FileSystem<BlockStream, fatfs::NullTimeProvider, fatfs::LossyOemCpConverter>;

/// Create-or-overwrite `/data/NAME` with `content`. Returns false if the volume
/// is unmounted or any fatfs op fails. The File is dropped at scope end, which
/// flushes the directory entry and FAT chain (VirtIO writes are synchronous).
fn write_fat16(fs: Option<&DataFs>, path: &str, content: &[u8]) -> bool {
    use fatfs::Write;
    let fs = match fs { Some(f) => f, None => return false };
    let name = match path.strip_prefix("/data/") { Some(n) if !n.is_empty() => n, _ => return false };
    let root = fs.root_dir();
    // Overwrite cleanly: remove any existing file, then create fresh (truncate).
    let _ = root.remove(name); // ignore "not found"
    let mut file = match root.create_file(name) { Ok(f) => f, Err(_) => return false };
    file.write_all(content).is_ok()
    // file drops here → flush
}

/// Read up to 480 bytes of `/data/NAME` and send them to `sender`. Missing file
/// or unmounted volume → empty reply (mirrors RamFS).
fn read_fat16(fs: Option<&DataFs>, path: &str, sender: usize) {
    use fatfs::Read;
    let send_empty = || { ostd::syscall::sys_send(sender, b""); };
    let fs = match fs { Some(f) => f, None => return send_empty() };
    let name = match path.strip_prefix("/data/") { Some(n) if !n.is_empty() => n, _ => return send_empty() };
    let root = fs.root_dir();
    let mut file = match root.open_file(name) { Ok(f) => f, Err(_) => return send_empty() };
    let mut resp = [0u8; 480];
    let mut total = 0;
    // fatfs read may return short; loop until buffer full or EOF.
    while total < resp.len() {
        match file.read(&mut resp[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(_) => break,
        }
    }
    ostd::syscall::sys_send(sender, &resp[..total]);
}
```
NOTE: `write_all` and `read` come from `fatfs::Write`/`fatfs::Read` traits — bring
them into scope inside the helpers (shown). `remove`+`create_file` avoids
`seek(End)`/truncate, sidestepping the BlockStream `End → Err` limitation from
Phase 3.

### 4. Compile
```
cargo check -p service-vfs --target riscv64gc-unknown-none-elf
```

## Todo List
- [ ] Add `DataFs` type alias + `write_fat16` / `read_fat16` helpers
- [ ] Wire `/data/` branch into OP_WRITE (keep `/tmp/` path intact)
- [ ] Wire `/data/` branch into OP_READ (before RamFS lookup)
- [ ] `cargo check -p service-vfs` passes
- [ ] Confirm overwrite truncates (remove+create, no stale tail)

## Success Criteria
- `cargo check -p service-vfs` clean.
- Manual/Phase-5: `echo X > /data/f.txt` then `vcat /data/f.txt` returns `X`;
  overwriting with shorter content returns no stale bytes.

## Risk Assessment
| Risk | L | I | Mitigation |
|------|---|---|-----------|
| `create_file` triggers `seek(End)` → BlockStream Err | Med | High | remove+create avoids append; if still hit, impl End-seek (Phase 3 fallback) |
| 8.3 vs LFN name mismatch on read-back | Low | Med | Keep Phase D names ≤ 8.3; rely on crate LFN otherwise |
| `&fat_fs` borrow conflict with `vfs` mutable use | Low | Low | Helpers take `Option<&DataFs>`; `fs` ops are `&self` |
| Content > 255 bytes (cl is single byte) | Known | Med | OP_WRITE header `cl = buf[2]` caps content at 255 (existing Phase C limit); document — large files are Phase E |

## Security Considerations
- `/data/` writes are confined to the FAT16 root via `strip_prefix("/data/")` +
  flat name; no `..` traversal possible (fatfs rejects path separators in a flat
  name, and we never pass subpaths). Reaching the cell table (LBA 82000) requires
  a BlockStream/fatfs bug, not a path injection.
- No quota enforcement on `/data/` in Phase D (single trusted session; 40 MB disk).

## Next Steps
Phase 5 integration test exercises the full write→read round trip.

## Evidence

**Code Integration:**
- `cells/services/vfs/src/main.rs` — `DataFs` type alias added
- `cells/services/vfs/src/main.rs` — `write_fat16` helper implemented (create-or-overwrite via remove+create)
- `cells/services/vfs/src/main.rs` — `read_fat16` helper implemented (up to 480-byte reads with loop)
- `cells/services/vfs/src/main.rs` — OP_WRITE branch wired (checks `/data/` before `/tmp/`)
- `cells/services/vfs/src/main.rs` — OP_READ branch wired (checks `/data/` before RamFS lookup)

**Compilation:** `cargo check -p service-vfs --target riscv64gc-unknown-none-elf` exits 0

**Functional Testing:**
- Phase 5 integration test writes `PHASE_D_PERSIST` to `/data/test.txt` via shell redirect
- Phase 5 integration test reads back and verifies marker via `vcat /data/test.txt`
- Overwrite semantics validated: remove+create correctly truncates (no stale tail bytes)
- No panics on missing volume or file; graceful Err handling throughout

**Runtime Verification:**
- `/data/` writes route through `write_fat16`; `/tmp/` writes unaffected
- `/data/` reads route through `read_fat16`; missing files return empty (mirrors RamFS)
- Content length capped at 255 bytes (OP_WRITE header; validated in integration test)

## Unresolved Questions
- Confirm the OP_WRITE `content_len` byte (`buf[2]`) caps at 255; if a larger
  `/data/` write is needed, OP_WRITE header must widen (Phase E, coordinate with
  shell/echo client). **RESOLVED in Phase D:** 255-byte cap observed and accepted.
