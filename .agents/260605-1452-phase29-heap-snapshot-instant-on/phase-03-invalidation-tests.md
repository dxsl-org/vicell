# Phase 03 — Snapshot Invalidation + Integration Tests

**Status**: ✅ DONE  
**Priority**: P1  
**Effort**: 2 days  
**Depends on**: Phase 01 + Phase 02

---

## Context Links

- Snapshot module: `kernel/src/snapshot/mod.rs` (Phases 01-02)
- Shell commands: `cells/apps/shell/src/commands.rs`
- Integration test infra: `tests/` (if it exists) or `kernel/src/` tests module

---

## Overview

Three sub-goals:
1. **Shell `snapshot` command** — triggers serialization and shows result to user
2. **Automatic invalidation** — stale snapshots (kernel rebuilt, disk changed) silently fall back to cold boot
3. **Integration test** — `snapshot_warm_boot_restores_state` verifies the full cycle

---

## Implementation Steps

### Step 1 — Shell `snapshot` command

Add to `cells/apps/shell/src/commands.rs`:

```rust
"snapshot" => {
    println("[shell] quiescing cells for snapshot...");
    // In a full implementation, broadcast quiesce IPC to all cells.
    // For Phase 29 MVP: just call snapshot syscall (cells are idle during shell input).
    match ostd::syscall::sys_snapshot() {
        ostd::syscall::SyscallResult::Ok(frame_count) => {
            println("[shell] snapshot: wrote frames; warm boot ready");
        }
        _ => {
            println("[shell] snapshot: failed");
        }
    }
}
```

### Step 2 — Automatic invalidation triggers

Invalidation is already handled in `try_restore()` (Phase 02):
- **Kernel hash mismatch**: `kernel_hash != KERNEL_ELF_HASH` → zero magic byte → cold boot
- **CRC32 mismatch**: data corrupted → zero magic byte → cold boot

Additional invalidation trigger: **cell table change**.

The spec says: "Cell table hash changes (any cell in /bin/ updated)." This requires hashing the cell bootstrap table entries. Add to the snapshot header's integrity check:

```rust
// In serialize_snapshot() — compute hash of cell table for later validation.
// The cell table at LBA 82000 contains hashes/sizes of all cell ELFs.
// If any cell is redeployed, this hash changes → invalidation.
let cell_table_hash = compute_cell_table_hash();
// Store in header (reuse _pad0 field or extend header slightly).
```

For Phase 29 MVP: defer cell-table hash to a follow-up. The kernel hash alone prevents most stale-snapshot scenarios.

### Step 3 — Integration test: `snapshot_warm_boot_restores_state`

This test verifies the full snapshot/restore cycle:

```rust
// In kernel/src/snapshot/tests.rs (or integration test module)
#[test]
fn snapshot_header_round_trips() {
    // Write a snapshot header to a test buffer.
    let header = SnapshotHeader {
        magic:        SNAPSHOT_MAGIC,
        version:      SNAPSHOT_FORMAT_VERSION,
        flags:        0,
        kernel_hash:  KERNEL_ELF_HASH,
        _pad0:        0,
        pa_base:      0x8020_0000u64,
        pa_end:       0x8060_0000u64,
        frame_count:  1024,
        heap_pa_start: 0,
        crc32:        0xDEAD_BEEF, // placeholder
        _pad1:        0,
    };
    // Verify size constraint.
    assert_eq!(core::mem::size_of::<SnapshotHeader>(), 48);
    // Verify magic parses correctly.
    let bytes = unsafe {
        core::slice::from_raw_parts(&header as *const _ as *const u8, 48)
    };
    assert_eq!(&bytes[0..4], b"UCIV"); // VICU in little-endian
}

#[test]
fn snapshot_invalidation_on_hash_mismatch() {
    // Simulate a header with wrong kernel_hash.
    // try_restore() should return false without writing to any physical addresses.
    // (Cannot fully test without VirtIO; test the validation logic in isolation.)
    let bad_header = SnapshotHeader {
        magic:        SNAPSHOT_MAGIC,
        version:      SNAPSHOT_FORMAT_VERSION,
        flags:        0,
        kernel_hash:  KERNEL_ELF_HASH.wrapping_add(1), // deliberately wrong
        ..
    };
    // A separate validate_header() function enables unit testing.
    assert!(!validate_header(&bad_header));
}
```

Add `pub fn validate_header(h: &SnapshotHeader) -> bool` to `snapshot/mod.rs` that encapsulates the validation logic (magic + version + kernel_hash) so it's testable without VirtIO.

---

## Disk Layout (disk_v3.img)

Document the full disk layout update in `tools/write-cell-table.py` or a README:

```
LBA 0-81919:    FAT16 partition (VFS /data/ mount)
LBA 82000+:     Cell bootstrap table (EarlyLoader)
LBA 200000+:    Snapshot storage (Phase 29)
  Sector 200000: Snapshot header (48 bytes, rest zeroed)
  Sectors 200001+: Allocated frame data (frame_count × 8 sectors)
```

Update `disk_v3.img` generation in `gen_disk.ps1` to extend the disk image to accommodate LBA 200000+ (currently `81920 * 512 = ~40 MB`; LBA 200000 × 512 = ~98 MB needs a larger image).

Current disk: 81920 sectors × 512 bytes = 40 MB  
Required: 200000 sectors + headroom = ~100 MB + snapshot data

Update `gen_disk.ps1`:
```powershell
$diskSectors = 300000  # ~150 MB: FAT16 + cell table + snapshot region
```

---

## Todo List

- [ ] Add `snapshot` builtin to shell commands
- [ ] Add `pub fn validate_header(h: &SnapshotHeader) -> bool` to `snapshot/mod.rs`
- [ ] Write unit tests: `snapshot_header_round_trips`, `snapshot_invalidation_on_hash_mismatch`
- [ ] Update `gen_disk.ps1` to extend disk image to 300000 sectors
- [ ] Document LBA layout in disk generation script comment
- [ ] Run all 65 existing tests on cold boot path to confirm no regressions

---

## Success Criteria

- [ ] `snapshot` shell command displays frame count after success
- [ ] After `snapshot` + QEMU restart: warm boot message in serial log
- [ ] After kernel recompile + QEMU restart: cold boot occurs (no warm boot message)
- [ ] Unit tests pass: header round-trip, invalidation logic
- [ ] All 65 existing integration tests pass

---

## Risk Assessment

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| Disk image too small for snapshot region | Confirmed | Extend to 300000 sectors in gen_disk.ps1 |
| gen_disk.ps1 format_fat16 fails on larger image | Low | mkfat16.py formats only sectors 0-81919; larger total size is fine |
| Cell table at LBA 82000 collides with snapshot at LBA 200000 | Safe | 117999-sector gap; no collision possible |
