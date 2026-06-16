# Phase 01 — Snapshot Serialization

**Status**: ✅ DONE  
**Priority**: P0  
**Effort**: 4 days

---

## Context Links

- Spec: `docs/specs/03-runtime.md §4`
- Frame allocator: `kernel/src/memory/frame.rs` — bitmap, `allocate_frame()`, iterate allocated frames
- VirtIO block: `kernel/src/task/drivers/virtio_blk.rs:132` — `write_sector(sector: u64, buf: &[u8])`
- Heap physical address: `kernel/src/main.rs:154-170` — dynamic allocation
- Kernel Cargo.toml: `kernel/Cargo.toml`
- Kernel build.rs: `kernel/build.rs` (currently only handles linker script + embedded cells)

---

## Overview

Implement `serialize_snapshot()` — enumerate all allocated physical frames (skipping MMIO and free frames), write the 48-byte header followed by frame data to a reserved LBA range on the disk.

A shell `snapshot` builtin triggers this via a new syscall or IPC to a kernel snapshot service.

---

## Key Constraints

- MMIO physical addresses (`< 0x80000000`) must be excluded — writing them would have side effects
- **ALL allocated physical frames must be included** — including kernel `.bss`/`.data` pages that hold global statics (SCHEDULER, BLOCK_DEVICE, FRAME_ALLOCATOR). These are in `.bss` but zeroed each boot; the snapshot restores their runtime values.
- Kernel `.text` / `.rodata` MAY be excluded (can be reloaded from ELF), but including them is safer and simpler for MVP. Recommend including them — the frame allocator marks them as "allocated" anyway.
- Total snapshot must be < 8 MB for acceptable performance (QEMU TCG 30 MB/s × 0.25s)

**Why BSS/data must be included (critical correctness):**
Global statics like `SCHEDULER: Spinlock<Option<Scheduler>>` live in `.bss`. BSS is re-zeroed by the boot process BEFORE `try_restore()` runs. If BSS frames are not in the snapshot, SCHEDULER is None when `yield_cpu()` is called → warm boot silently fails after overwriting all of RAM. Including BSS frames ensures SCHEDULER (and all other globals) are restored to their runtime state.

---

## Related Code Files

### Modify
- `kernel/Cargo.toml` — add `crc32fast = { version = "1", default-features = false }`
- `kernel/build.rs` — add `KERNEL_ELF_HASH` emit (CRC32 of kernel binary)

### Create
- `kernel/src/snapshot/mod.rs` — `serialize_snapshot()`, `SnapshotHeader` struct

### Modify
- `kernel/src/task/syscall.rs` — add `Syscall::Snapshot` handler
- `libs/api/src/syscall.rs` — add `ViSyscall::Snapshot = 420`
- `libs/ostd/src/syscall.rs` — add `sys_snapshot()` wrapper

---

## Implementation Steps

### Step 1 — Add `crc32fast` to `kernel/Cargo.toml`

```toml
crc32fast = { version = "1", default-features = false }
```

### Step 2 — Emit kernel hash from `build.rs`

**⚠️ Red-team fix**: Reading the kernel ELF binary in `build.rs` is a chicken-and-egg problem — the binary doesn't exist on the first build, and on subsequent builds it reads the PREVIOUS binary (always one build stale). Use `vergen-gitcl` to emit the Git SHA instead — this changes on every git commit and is available without reading the binary.

```toml
# kernel/Cargo.toml [build-dependencies]
vergen-gitcl = { version = "1", features = ["build"] }
```

```rust
// kernel/build.rs
fn main() {
    // ... existing linker script + embedded cell code ...

    // Emit the Git commit SHA as the kernel invalidation key.
    // Any commit that changes kernel code (or cells) produces a different SHA,
    // invalidating any existing snapshot and forcing a cold boot.
    use vergen_gitcl::{BuildBuilder, CargoBuilder, Emitter, GitclBuilder};
    let git = GitclBuilder::default()
        .sha(false)  // short SHA is fine
        .build()
        .unwrap_or_default();
    Emitter::default()
        .add_instructions(&git)
        .unwrap_or_default()
        .emit()
        .unwrap_or_default();
    // Fallback if not in a git repo:
    if std::env::var("VERGEN_GIT_SHA").is_err() {
        println!("cargo:rustc-env=VERGEN_GIT_SHA=000000000");
    }
}
```

```rust
// kernel/src/snapshot/mod.rs — parse SHA as a u64 hash
pub const KERNEL_GIT_SHA: &str = env!("VERGEN_GIT_SHA");

fn kernel_hash_from_sha() -> u64 {
    // Parse first 8 hex chars of SHA → u64 for header field.
    u64::from_str_radix(&KERNEL_GIT_SHA[..8.min(KERNEL_GIT_SHA.len())], 16)
        .unwrap_or(0)
}
```

### Step 3 — Create `kernel/src/snapshot/mod.rs`

```rust
//! Heap snapshot serialization for warm-boot instant-on.
//!
//! Writes all allocated physical frames to a reserved sector range on the
//! VirtIO block device.  The 48-byte header allows the boot path to detect,
//! validate, and restore the snapshot without heap-level initialization.

use crate::memory::frame::FRAME_ALLOCATOR;
use crate::task::drivers::virtio_blk::viVirtIOBlk;
use api::block::ViBlockDevice;

/// Reserved LBA range for snapshot storage in disk_v3.img.
/// Placed well beyond the cell bootstrap table at LBA 82000.
pub const SNAPSHOT_BASE_LBA: u64 = 200_000;

/// Snapshot format version — increment on header layout changes.
pub const SNAPSHOT_FORMAT_VERSION: u16 = 1;

/// Magic bytes identifying a ViCell snapshot image ("VICU" LE).
pub const SNAPSHOT_MAGIC: u32 = 0x5649_4355;

/// Kernel ELF CRC32 baked in at compile time via build.rs.
pub const KERNEL_ELF_HASH: u32 = {
    let s = env!("KERNEL_ELF_HASH");
    // Parse decimal string to u32 at compile time.
    let mut result = 0u32;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        result = result.wrapping_mul(10).wrapping_add((bytes[i] - b'0') as u32);
        i += 1;
    }
    result
};

/// 48-byte snapshot header (see docs/specs/03-runtime.md §4).
#[repr(C)]
pub struct SnapshotHeader {
    pub magic:        u32,
    pub version:      u16,
    pub flags:        u16,
    pub kernel_hash:  u32,
    pub _pad0:        u32,
    pub pa_base:      u64,
    pub pa_end:       u64,
    pub frame_count:  u32,
    pub heap_pa_start: u32,
    pub crc32:        u32,
    pub _pad1:        u32,
}

// Compile-time size check — `const_assert!` is not in core; use const assert instead.
const _: () = assert!(core::mem::size_of::<SnapshotHeader>() == 48);

/// Serialize all allocated physical frames to the reserved disk sector range.
///
/// Must be called with all cells quiesced (at a scheduler yield point so no
/// task stack is mid-function-call).  Excludes MMIO regions and free frames.
///
/// Returns the number of frames written, or an error.
pub fn serialize_snapshot() -> Result<u32, &'static str> {
    let allocator_guard = FRAME_ALLOCATOR.lock();
    let allocator = allocator_guard.as_ref().ok_or("frame allocator not initialized")?;

    let pa_base = allocator.memory_start;
    let pa_end  = allocator.memory_end;

    // Enumerate allocated frames; collect their physical addresses.
    // We write frames sequentially — the header records pa_base so the
    // restore path knows where to place them.
    let mut current_lba = SNAPSHOT_BASE_LBA + 1; // sector 0 = header
    let mut frame_count = 0u32;
    let mut hasher = crc32fast::Hasher::new();

    // Walk all frames in the allocatable region.
    let total_frames = (pa_end - pa_base) / 4096;
    for frame_idx in 0..total_frames {
        if !allocator.is_frame_allocated(frame_idx) { continue; }

        let pa = pa_base + frame_idx * 4096;
        // Exclude MMIO (physical addresses below RAM base 0x80000000).
        if pa < 0x8000_0000 { continue; }

        // Write 8 sectors per 4096-byte frame.
        let frame_bytes = unsafe {
            // SAFETY: pa is a valid allocated physical frame within RAM;
            // the frame allocator owns it and no cell is writing to it
            // during the quiesced snapshot window.
            core::slice::from_raw_parts(pa as *const u8, 4096)
        };
        for sector_offset in 0..8 {
            let sector_bytes = &frame_bytes[sector_offset * 512..(sector_offset + 1) * 512];
            let mut buf = [0u8; 512];
            buf.copy_from_slice(sector_bytes);
            viVirtIOBlk.write_sector(current_lba, &buf)
                .map_err(|_| "write_sector failed")?;
            hasher.update(sector_bytes);
            current_lba += 1;
        }
        frame_count += 1;
    }

    // Write header to sector 0 of snapshot region.
    let final_crc = hasher.finalize();
    let header = SnapshotHeader {
        magic:        SNAPSHOT_MAGIC,
        version:      SNAPSHOT_FORMAT_VERSION,
        flags:        0,
        kernel_hash:  KERNEL_ELF_HASH,
        _pad0:        0,
        pa_base:      pa_base as u64,
        pa_end:       pa_end as u64,
        frame_count,
        heap_pa_start: 0, // TODO: record heap start offset
        crc32:        final_crc,
        _pad1:        0,
    };
    let header_bytes = unsafe {
        // SAFETY: SnapshotHeader is repr(C) with known layout; transmuting to
        // bytes for sector write is safe.
        core::slice::from_raw_parts(
            &header as *const SnapshotHeader as *const u8,
            core::mem::size_of::<SnapshotHeader>(),
        )
    };
    let mut header_sector = [0u8; 512];
    header_sector[..48].copy_from_slice(header_bytes);
    viVirtIOBlk.write_sector(SNAPSHOT_BASE_LBA, &header_sector)
        .map_err(|_| "write header sector failed")?;

    log::info!("[snapshot] wrote {} frames ({} MiB) to LBA {}",
        frame_count,
        frame_count * 4096 / 1024 / 1024,
        SNAPSHOT_BASE_LBA);
    Ok(frame_count)
}
```

---

## Todo List

- [ ] Add `crc32fast = { version = "1", default-features = false }` to `kernel/Cargo.toml`
- [ ] Update `build.rs` to emit `KERNEL_ELF_HASH`
- [ ] Create `kernel/src/snapshot/mod.rs` with `serialize_snapshot()`
- [ ] Add `pub mod snapshot;` to `kernel/src/main.rs`
- [ ] Add `is_frame_allocated(frame_idx: usize) -> bool` method to `FrameAllocator`
- [ ] Add `ViSyscall::Snapshot = 420` to `libs/api/src/syscall.rs`
- [ ] Add `sys_snapshot()` to `libs/ostd/src/syscall.rs`
- [ ] Add `Syscall::Snapshot` handler in `kernel/src/task/syscall.rs`
- [ ] Add `snapshot` builtin to shell
- [ ] `cargo check --target riscv64gc-unknown-none-elf -Z build-std=core,alloc -p vicell-kernel` — clean

---

## Success Criteria

- [ ] `snapshot` command creates sectors at LBA 200000
- [ ] Header reads back with correct magic, version, kernel_hash, crc32
- [ ] Frame count matches number of allocated frames in bitmap
- [ ] MMIO addresses (< 0x80000000) not present in written sectors
