# Phase 02 — Warm Boot Restore

**Status**: ✅ DONE  
**Priority**: P0  
**Effort**: 4 days  
**Depends on**: Phase 01

---

## Context Links

- Boot sequence: `kernel/src/main.rs:39-291` — `kmain()`
- VirtIO init: `kernel/src/task/drivers/virtio_blk.rs:19` — `init_driver()`
- EarlyLoader probe: `kernel/src/loader/early.rs` — `EarlyLoader::probe()`
- Snapshot constants: `kernel/src/snapshot/mod.rs` (Phase 01)
- Frame allocator: `kernel/src/memory/frame.rs`

---

## Overview

Insert `snapshot::try_restore()` between VirtIO initialization and EarlyLoader cell loading. On a valid snapshot:
1. Read header → validate magic, version, kernel_hash, crc32
2. `memcpy` allocated frames from disk sectors to original physical addresses
3. Reinitialize all hardware (PLIC, VirtIO — hardware state cannot be snapshotted)
4. Restore CPU state from snapshot header
5. Skip EarlyLoader and cell spawning (cells resume from snapshot)

On invalid/absent snapshot: cold boot as before.

---

## Restore Sequence (Concrete Boot Order)

```
kmain():
  1. UART init
  2. HAL init (trap setup)
  3. parse_bootloader_info() → Limine/OpenSBI boot info
  4. FrameAllocator::new_from_map(mmap_entries)
  5. init_kernel_paging()
  6. activate_paging()
  7. init_heap()           ← heap is live
  8. rt_heap::init()
  9. PLIC init
  10. task::drivers::init()  ← VirtIO live (needed to read snapshot)
  11. [NEW] snapshot::try_restore()  ← warm boot path OR cold boot continues
      If warm boot:
        a. Read header from SNAPSHOT_BASE_LBA
        b. Validate → fail → continue cold boot
        c. Read frame data → write to original PAs
        d. Reinit PLIC + VirtIO (hardware reset)
        e. Resume: tasks already in scheduler, yield_cpu()
        (cold boot steps 12-17 are SKIPPED)
  12. [Cold boot] EarlyLoader::probe()
  13. [Cold boot] fs::init()
  14. [Cold boot] task::init()
  15. [Cold boot] spawn init cell
```

---

## Related Code Files

### Modify
- `kernel/src/main.rs` — insert `snapshot::try_restore()` between steps 10 and 12
- `kernel/src/snapshot/mod.rs` — add `try_restore()` function

---

## Implementation Steps

### Step 1 — Add `try_restore()` to `kernel/src/snapshot/mod.rs`

```rust
/// Attempt to restore the kernel from a previously written snapshot.
///
/// Returns `true` if warm boot succeeded (caller should skip cell init).
/// Returns `false` if no valid snapshot found or validation fails (cold boot).
///
/// Must be called after VirtIO block driver is initialized and before
/// EarlyLoader::probe() or task scheduler init.
pub fn try_restore() -> bool {
    // Read snapshot header sector.
    let mut header_sector = [0u8; 512];
    if viVirtIOBlk.read_sector(SNAPSHOT_BASE_LBA, &mut header_sector).is_err() {
        log::info!("[snapshot] no block device → cold boot");
        return false;
    }

    // Parse header.
    let header: &SnapshotHeader = unsafe {
        // SAFETY: header_sector is 512 bytes; SnapshotHeader is 48 bytes repr(C);
        // alignment is satisfied (u8 array, no strict alignment needed for cast).
        &*(header_sector.as_ptr() as *const SnapshotHeader)
    };

    // Validate magic.
    if header.magic != SNAPSHOT_MAGIC {
        log::info!("[snapshot] magic mismatch → cold boot");
        return false;
    }
    // Validate format version.
    if header.version != SNAPSHOT_FORMAT_VERSION {
        log::info!("[snapshot] format version mismatch ({} != {}) → cold boot",
            header.version, SNAPSHOT_FORMAT_VERSION);
        return false;
    }
    // Validate kernel build hash — invalidates on recompile.
    if header.kernel_hash != KERNEL_ELF_HASH {
        log::info!("[snapshot] kernel changed → cold boot (invalidating snapshot)");
        // Zero out magic to prevent future false hits on this stale snapshot.
        invalidate_snapshot();
        return false;
    }

    let frame_count = header.frame_count as usize;
    let pa_base     = header.pa_base as usize;

    // Verify CRC32 over header (crc32 field = 0 during calc) + all frame data.
    let saved_crc = header.crc32;
    {
        let mut hasher = crc32fast::Hasher::new();
        // Hash header with crc32 field zeroed.
        let mut header_copy = header_sector;
        header_copy[40..44].copy_from_slice(&[0u8; 4]); // zero crc32 field
        hasher.update(&header_copy[..48]);

        // Hash all frame sectors.
        let mut frame_lba = SNAPSHOT_BASE_LBA + 1;
        let mut buf = [0u8; 512];
        for _ in 0..frame_count * 8 {
            if viVirtIOBlk.read_sector(frame_lba, &mut buf).is_err() {
                log::warn!("[snapshot] read error during CRC check → cold boot");
                return false;
            }
            hasher.update(&buf);
            frame_lba += 1;
        }
        if hasher.finalize() != saved_crc {
            log::warn!("[snapshot] CRC32 mismatch → cold boot (snapshot corrupted)");
            invalidate_snapshot();
            return false;
        }
    }

    log::info!("[snapshot] valid snapshot: {} frames at PA 0x{:X} → restoring",
        frame_count, pa_base);

    // Read frame data and write directly to original physical addresses.
    let mut frame_lba = SNAPSHOT_BASE_LBA + 1;
    for frame_idx in 0..frame_count {
        let pa = pa_base + frame_idx * 4096;
        let frame_ptr = pa as *mut u8;
        for sector_offset in 0..8usize {
            let mut buf = [0u8; 512];
            if viVirtIOBlk.read_sector(frame_lba, &mut buf).is_err() {
                log::error!("[snapshot] read failed at frame {} → cold boot", frame_idx);
                return false;
            }
            // SAFETY: pa is within RAM (pa_base >= 0x80000000, validated above by CRC
            // and the fact that we wrote these frames in serialize_snapshot which checked
            // PA >= 0x80000000).  frame_ptr + sector_offset * 512 is within the 4096-byte frame.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    buf.as_ptr(),
                    frame_ptr.add(sector_offset * 512),
                    512,
                );
            }
            frame_lba += 1;
        }
    }

    log::info!("[snapshot] frames restored → reinitializing hardware");

    // At this point, ALL physical frames (including kernel .bss/.data pages) have
    // been overwritten with snapshot data.  Global statics like SCHEDULER,
    // BLOCK_DEVICE, and FRAME_ALLOCATOR now contain their runtime values from
    // the time the snapshot was taken.  This is why BSS frames MUST be included
    // in the snapshot — otherwise SCHEDULER would still be None (re-zeroed at boot).

    // Re-initialize frame allocator to match the restored bitmap state.
    // The restored FRAME_ALLOCATOR global has the old bitmap, but the bitmap pages
    // themselves are now restored correctly at their original PAs.
    // Calling new_from_map again is INCORRECT (would reinit bitmap from scratch).
    // Instead: trust the restored global — it's valid since we snaphotted the bitmap.
    // Just verify the guard: FRAME_ALLOCATOR lock should be in unlocked state.
    // (The spinlock word in BSS was restored; it was unlocked when snapshot was taken.)

    // Reinitialize hardware (MMIO state resets on every power cycle).
    // VirtIO must be re-registered because device-side queue registers need
    // to be replayed even though the Rust struct in heap is now restored.
    #[cfg(target_arch = "riscv64")]
    crate::hal::common::plic::init();

    // Re-run VirtIO init — overwrites BLOCK_DEVICE global with a newly-initialized
    // device.  The restored BLOCK_DEVICE from snapshot is discarded (correct: hardware
    // was reset, descriptor ring pointers in old struct are stale after device reset).
    crate::task::drivers::virtio_blk::init_driver();
    crate::task::drivers::virtio_net::init_driver();

    // Re-arm timer.
    #[cfg(target_arch = "riscv64")]
    {
        let next = hal::common::timer::read_mtime() + hal::common::timer::TICKS_PER_10MS;
        hal::common::sbi::set_timer(next);
    }

    log::info!("[snapshot] warm boot complete → resuming scheduler");

    // SCHEDULER is now Some(restored_scheduler) with tasks at their last yield points.
    // yield_cpu() will pick the next ready task and context-switch into it.
    // This works ONLY because BSS frames (containing SCHEDULER) were included in snapshot.
    crate::task::yield_cpu();

    // yield_cpu() will return if no tasks are ready (shouldn't happen on warm boot).
    // Fall through to cold boot as a safety net.
    false
}

/// Zero out the snapshot magic to force cold boot on next restart.
fn invalidate_snapshot() {
    let mut buf = [0u8; 512];
    let _ = viVirtIOBlk.write_sector(SNAPSHOT_BASE_LBA, &buf);
}
```

### Step 2 — Insert in `main.rs`

After `task::drivers::init()` (line ~203) and before `EarlyLoader::probe()` (line ~208):

```rust
// Attempt warm boot from snapshot before any cell spawning.
// Returns true if snapshot was valid and scheduler resumed.
if crate::snapshot::try_restore() {
    // try_restore() called yield_cpu() which should not return in warm boot.
    // If we reach here, fall through to cold boot.
    log::warn!("[boot] warm boot returned unexpectedly — continuing cold boot");
}
```

---

## Cell Quiescence Protocol

Cells must be at a `yield_cpu()` point when `serialize_snapshot()` is called (Phase 01). This ensures no cell stack frame is mid-execution with a corrupted stack pointer on restore.

The shell `snapshot` command should:
1. Send a broadcast IPC "quiesce" to all cells
2. Each cell calls `yield_cpu()` after receiving the message
3. Shell calls `sys_snapshot()` kernel syscall
4. Kernel's snapshot handler sees all other cells blocked on Recv — safe to snapshot

---

## Todo List

- [ ] Add `try_restore()` to `kernel/src/snapshot/mod.rs`
- [ ] Add `invalidate_snapshot()` helper
- [ ] Insert `snapshot::try_restore()` call in `kernel/src/main.rs` between steps 10-12
- [ ] Add VirtIO net reinit call in restore path (if net driver has `init_driver()`)
- [ ] Test: after `snapshot` + reboot, kernel log shows warm boot message
- [ ] Test: after `snapshot` + kernel recompile + reboot, cold boot occurs (hash mismatch)

---

## Success Criteria

- [ ] Valid snapshot → `[snapshot] warm boot complete` in serial log
- [ ] Magic mismatch → `[snapshot] magic mismatch → cold boot` + normal cold boot
- [ ] Kernel hash mismatch → `[snapshot] kernel changed → cold boot` + snapshot invalidated
- [ ] CRC32 corruption → `[snapshot] CRC32 mismatch → cold boot`
- [ ] All 65 integration tests pass on warm boot path

---

## Risk Assessment

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| Cell stack mid-execution on snapshot | Medium | Quiescence protocol (all cells at yield point) |
| BLOCK_DEVICE stale after restore (VirtIO init resets global) | Confirmed | Re-run `init_driver()` after restore — documented in impl |
| pa_base varies across reboots (QEMU memory map non-deterministic) | Low | QEMU virt always reports same memory map; add assert if needed |
| Frame allocator bitmap (at pa_base) overwritten during restore | Low | Bitmap is included in snapshot frames; restored correctly before allocator reuse |
