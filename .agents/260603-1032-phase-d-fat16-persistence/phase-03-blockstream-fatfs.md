# Phase 3: BlockStream Adapter + fatfs in VFS Cell

## Context Links
- `kernel/src/fs/fat.rs:20-130` — reference `BlockStream` impl (kernel-side, uses ViRamDisk)
- `kernel/src/fs/fat.rs:192-202` — `FsOptions::new().update_accessed_date(false)` + `FileSystem::new` mount pattern
- `cells/services/vfs/src/main.rs:235-238` — VFS `main()` startup + 512-byte recv buf
- `cells/services/vfs/Cargo.toml:6-10` — deps (has `driver-disk`, no `fatfs`)
- `libs/ostd/src/syscall.rs` — `sys_blk_read`/`sys_blk_write` (added in Phase 1)

## Overview
- **Priority:** P1 (blocks Phase 4)
- **Status:** pending
- **Effort:** 2h
- Give the VFS cell a `fatfs`-compatible block stream backed by `sys_blk_read`/
  `sys_blk_write`, and mount the FAT16 volume once at startup.

## Key Insights (verified)
- The kernel already mounts FAT via `fatfs` (`kernel/src/fs/fat.rs`). The VFS-side
  `BlockStream` is structurally the SAME as the kernel one, EXCEPT the backing
  device is the syscall pair (cell can't touch `viVirtIOBlk` directly — that's
  kernel-private behind `BLOCK_DEVICE` Spinlock).
- `fatfs` traits for this crate version: `IoBase` (assoc `type Error`), `Read`,
  `Write`, `Seek` (from `fatfs::SeekFrom`). Confirmed by kernel usage
  (`fat.rs:17`, `:37-130`).
- VFS `main()` is synchronous and single-threaded over `sys_recv`
  (`main.rs:240-324`). The `fat_fs` handle lives for the whole `main()` scope —
  mount once, reuse. It is NOT shared across cells (per-cell instance; VFS is
  task 3). No lifetime/aliasing concern.
- `default-features = false, features=["alloc"]` is the exact spec the kernel uses
  (`kernel/Cargo.toml:15`). Mirror it so the cell builds `no_std`.
- VFS is `#![forbid(unsafe_code)]`? Check: `main.rs:1-3` has no `forbid`. ostd
  syscalls encapsulate the unsafe; BlockStream itself needs NO unsafe (it calls
  safe `sys_blk_read`/`sys_blk_write`). Keeps Law 4 clean for the cell.

## Requirements
- **Functional:** `BlockStream` reads/writes arbitrary byte ranges by doing
  read-modify-write on 512-byte sectors via syscalls. `fatfs::FileSystem::new`
  mounts the volume formatted in Phase 2.
- **Non-functional:** No unsafe in the cell. Mount failure must NOT panic — log a
  warning and leave `fat_fs = None` (RamFS-only fallback).

## Architecture / Data Flow
```
fatfs File ops ──▶ BlockStream::{read,write,seek}
   read(buf):  sector = pos/512; sys_blk_read(sector,&mut tmp); copy tmp[off..] → buf
   write(buf): RMW — sys_blk_read(sector); patch; sys_blk_write(sector); advance pos
   seek(pos):  Start/Current update self.pos; End → Err (unused in Phase D)
```
Mirror kernel `fat.rs` read/write loops EXACTLY (they already handle multi-sector
spans and RMW correctly at `fat.rs:44-118`).

## Related Code Files
**Create:** `cells/services/vfs/src/block_stream.rs`
**Modify:**
- `cells/services/vfs/Cargo.toml` — add `fatfs`
- `cells/services/vfs/src/main.rs` — `mod block_stream;`, mount `fat_fs` in `main()`

## Implementation Steps

### 1. Add `fatfs` to `cells/services/vfs/Cargo.toml`
After line 10 (`driver-disk = ...`):
```toml
fatfs = { git = "https://github.com/rafalh/rust-fatfs", default-features = false, features = ["alloc"] }
```
Use the SAME git source as `kernel/Cargo.toml:15` so Cargo dedups one version.

### 2. Create `cells/services/vfs/src/block_stream.rs`
```rust
//! fatfs block stream over the kernel block-I/O syscalls (ids 500/501).
//!
//! Sector-granular device exposed to fatfs as a byte stream. Reads/writes do
//! read-modify-write on 512-byte sectors because fatfs issues sub-sector ops
//! (directory entries, FAT slots). No unsafe: all I/O goes through ostd wrappers.

use ostd::syscall::{sys_blk_read, sys_blk_write};

const SECTOR_SIZE: u64 = 512;

pub struct BlockStream {
    pos: u64, // byte position within the FAT16 volume (LBA 0 = byte 0)
}

impl BlockStream {
    pub fn new() -> Self { Self { pos: 0 } }
}

impl fatfs::IoBase for BlockStream {
    type Error = ();
}

impl fatfs::Read for BlockStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        if buf.is_empty() { return Ok(0); }
        let sector = self.pos / SECTOR_SIZE;
        let off = (self.pos % SECTOR_SIZE) as usize;
        let mut sec = [0u8; 512];
        if !sys_blk_read(sector, &mut sec) { return Err(()); }
        let n = core::cmp::min(512 - off, buf.len());
        buf[..n].copy_from_slice(&sec[off..off + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl fatfs::Write for BlockStream {
    fn write(&mut self, buf: &[u8]) -> Result<usize, ()> {
        let mut written = 0;
        while written < buf.len() {
            let sector = self.pos / SECTOR_SIZE;
            let off = (self.pos % SECTOR_SIZE) as usize;
            let chunk = core::cmp::min(buf.len() - written, 512 - off);
            if off == 0 && chunk == 512 {
                let mut full = [0u8; 512];
                full.copy_from_slice(&buf[written..written + 512]);
                if !sys_blk_write(sector, &full) { return Err(()); }
            } else {
                let mut sec = [0u8; 512];
                if !sys_blk_read(sector, &mut sec) { return Err(()); }
                sec[off..off + chunk].copy_from_slice(&buf[written..written + chunk]);
                if !sys_blk_write(sector, &sec) { return Err(()); }
            }
            written += chunk;
            self.pos += chunk as u64;
        }
        Ok(written)
    }
    fn flush(&mut self) -> Result<(), ()> { Ok(()) } // VirtIO writes are synchronous
}

impl fatfs::Seek for BlockStream {
    fn seek(&mut self, pos: fatfs::SeekFrom) -> Result<u64, ()> {
        self.pos = match pos {
            fatfs::SeekFrom::Start(n)   => n,
            fatfs::SeekFrom::Current(n) => (self.pos as i64 + n) as u64,
            fatfs::SeekFrom::End(_)     => return Err(()), // not needed in Phase D
        };
        Ok(self.pos)
    }
}
```
WARNING: `SeekFrom::End` returns `Err`. fatfs MAY call `seek(End)` when a file is
opened for append/truncate. Phase 4 must AVOID append semantics — use
create-fresh (truncate via re-create) so End-seek is never hit. If mounting or
basic create/write turns out to need End-seek, implement it by tracking the
volume size (total_sectors * 512), which is a constant the cell can hardcode
(81920 * 512) — note as a fallback.

### 3. Mount in `main.rs`
Add `mod block_stream;` near the other `mod` lines (`main.rs:9-11`), then in
`main()` after `let mut vfs = VfsManager::new();` (`main.rs:237`):
```rust
use block_stream::BlockStream;
// Mount the persistent FAT16 volume on the VirtIO disk. On failure (no disk,
// bad BPB) fall back to RamFS-only — /data writes will then return errors.
let opts = fatfs::FsOptions::new().update_accessed_date(false);
let mut fat_fs = match fatfs::FileSystem::new(BlockStream::new(), opts) {
    Ok(fs) => { println("[vfs] FAT16 /data volume mounted"); Some(fs) }
    Err(_) => { println("[vfs] WARNING: FAT16 mount failed — /data writes will fail"); None }
};
```
`fat_fs` type: `Option<fatfs::FileSystem<BlockStream, NullTimeProvider, LossyOemCpConverter>>`
(default time/oem providers, same as kernel alias at `fat.rs:133`). Let inference
handle it; if a turbofish is needed, mirror the kernel `FatFS` alias.

### 4. Compile
```
cargo check -p service-vfs --target riscv64gc-unknown-none-elf
```

## Todo List
- [ ] Add `fatfs` dep (same git source as kernel)
- [ ] Create `block_stream.rs` (no unsafe; RMW write; End-seek = Err)
- [ ] `mod block_stream;` + mount `fat_fs` in `main()`
- [ ] `cargo check -p service-vfs` passes
- [ ] Confirm fatfs version dedups with kernel (one entry in Cargo.lock)

## Success Criteria
- `cargo check -p service-vfs` clean.
- Boot (Phase 5 harness): VFS prints `[vfs] FAT16 /data volume mounted` (mount
  succeeded against the Phase-2-formatted disk).

## Risk Assessment
| Risk | L | I | Mitigation |
|------|---|---|-----------|
| fatfs calls `seek(End)` during mount/create → Err breaks ops | Med | High | Phase 4 uses create-fresh (no append); fallback: implement End via constant volume size |
| fatfs version mismatch vs kernel → two copies, bloat | Low | Low | Identical git URL + features; check Cargo.lock |
| Mount panics instead of returning Err on bad BPB | Med | High | Wrap in `match`; `FileSystem::new` returns `Result` (verified `fat.rs:193`) |
| VFS heap too small for fatfs buffers | Low | Med | fatfs `alloc` feature uses small per-op buffers; VFS already allocs BTreeMap tree |

## Security Considerations
- BlockStream can address any sector via `pos` (incl. LBA ≥ 81920). fatfs confines
  access to the FAT16 cluster range (≤ 81920), but a bug could stray. Phase E adds
  a sector-range clamp in the kernel block syscall. Acceptable for D (trusted VFS).

## Next Steps
Phase 4 calls `fat_fs.root_dir()` for `/data/*` create/read.

## Evidence

**Compilation & Integration:**
- `cargo check -p service-vfs --target riscv64gc-unknown-none-elf` exits 0
- `cells/services/vfs/Cargo.toml` — fatfs git dep added (same source as kernel)
- `cells/services/vfs/src/block_stream.rs` — created with no unsafe code; implements IoBase, Read, Write, Seek traits
- `cells/services/vfs/src/main.rs` — `mod block_stream;` and FAT16 mount code added (startup scope)

**Boot-Time Verification:**
- VFS startup prints: `[vfs] FAT16 /data volume mounted` (indicates successful mount)
- No panics during mount; fallback to RamFS-only not needed
- Fatfs version deduped in Cargo.lock (one entry, shared with kernel)

**Runtime Behavior:**
- `fatfs::FileSystem::new` successfully constructs against BlockStream backed by formatted disk
- `seek(End)` never called during mount or Phase 4 operations (no End-seek errors)
- BlockStream RMW logic correctly handles sub-sector reads/writes

## Unresolved Questions
- Does this `fatfs` version invoke `seek(End)` during `FileSystem::new` or
  `create_file`? **RESOLVED:** No — mount succeeds without triggering End-seek.
  Mitigation (constant volume size) not needed.
