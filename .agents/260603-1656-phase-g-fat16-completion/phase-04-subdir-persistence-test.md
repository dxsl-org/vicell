# Phase 4: Subdir Reboot Persistence Test

## Context Links
- `tests/integration/tests/boot.rs:364-399+` — `vfs_fat16_reboot_persistence` (pattern to mirror)
- `tests/integration/tests/boot.rs:33` — `disk_path()` → `disk_v3.img` (shared across boots)
- `tests/integration/tests/boot.rs:13,15` — `BOOT_TIMEOUT=40`, `CMD_TIMEOUT=10`
- `tests/integration/src/lib.rs:139,155,174,179` — `send_line`, `wait_for_natural_exit`, `output_contains`, `dump`
- `cells/services/vfs/src/main.rs:415-423` — OP_MKDIR `/data/` → `fat16_mkdir` (subdir write path)
- `cells/services/vfs/src/main.rs:386-402` — OP_WRITE `/data/` → `write_fat16` + flush

## Overview
- **Priority:** P2
- **Status:** pending
- **Description:** Add `vfs_fat16_subdir_persistence` — a test proving a write into a FAT16
  SUBDIRECTORY survives a full reboot. Test-only; the write/flush code is already correct
  from Phases E + F.
- **Independent** of Phases 1–3 (no production code change).

## Key Insights (verified 2026-06-03)
- Phase E's `vfs_fat16_reboot_persistence` (boot.rs:364) is the exact template: boot → write
  marker → `shutdown` → wait `"System shutting down"` → `wait_for_natural_exit(15)` → reboot
  → assert marker via `vcat`.
- **CORRECTION:** The brief's sketch omitted the `wait_for("System shutting down")` step that
  the real Phase E test performs (boot.rs:391) before `wait_for_natural_exit`. Mirror it.
- Subdir path: `mkdir /data/pdir` (OP_MKDIR → `fat16_mkdir` → `ensure_dir_chain`, main.rs:418)
  then `echo X > /data/pdir/f.txt` (OP_WRITE → `write_fat16`, main.rs:393, which calls
  `ensure_dir_chain` for the parent then `BlockStream::flush`). Both paths verified present.
- `disk_v3.img` is shared between both boots; the write is create-or-overwrite (idempotent),
  so re-runs are safe — same property the Phase E test relies on.
- The marker string must be unique to avoid `wait_for` matching a stale earlier line. Use
  `SUBDIR_PERSIST`.

## Data Flow
```
BOOT 1:  mkdir /data/pdir ─▶ OP_MKDIR ─▶ fat16_mkdir ─▶ ensure_dir_chain
         echo SUBDIR_PERSIST > /data/pdir/f.txt ─▶ OP_WRITE ─▶ write_fat16
                                                              └▶ BlockStream::flush ─▶ VirtIO FLUSH ─▶ disk_v3.img
         shutdown ─▶ "System shutting down" ─▶ QEMU clean exit (image flushed)
                                  │
                          (same disk_v3.img)
                                  ▼
BOOT 2:  vcat /data/pdir/f.txt ─▶ OP_READ ─▶ read_fat16 ─▶ "SUBDIR_PERSIST"  ✓
```

## Related Code Files
**Modify:** `tests/integration/tests/boot.rs` (append one `#[test]` fn). **Create/Delete:** none.

## Implementation Steps

### Append the test (boot.rs)
Append after `vfs_fat16_reboot_persistence` (append-only; no line overlap with Phase 3):
```rust
/// Phase G: a FAT16 SUBDIRECTORY write survives a full reboot.
///
/// Same power-cycle pattern as `vfs_fat16_reboot_persistence`, but the marker is
/// written into a nested dir (`/data/pdir/f.txt`) created at runtime via `mkdir`.
/// Shares `disk_v3.img`; create-or-overwrite keeps re-runs safe.
#[test]
fn vfs_fat16_subdir_persistence() {
    if !prerequisites_ok() {
        return;
    }

    // ── First boot: mkdir + write into the subdir, then shut down ──────────────
    let mut qemu = QemuRunner::boot(&kernel_path(), &disk_path());
    qemu.wait_for("ViCell >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("first boot prompt failed: {e}\n{}", qemu.dump()));
    assert!(
        qemu.output_contains("FAT16 /data volume mounted"),
        "FAT16 not mounted on first boot\n{}", qemu.dump()
    );
    std::thread::sleep(std::time::Duration::from_millis(500));

    qemu.send_line("mkdir /data/pdir");
    qemu.wait_for("ViCell >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("mkdir did not return to prompt: {e}\n{}", qemu.dump()));

    qemu.send_line("echo SUBDIR_PERSIST > /data/pdir/f.txt");
    qemu.wait_for("ViCell >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("subdir write did not return to prompt: {e}\n{}", qemu.dump()));

    qemu.send_line("shutdown");
    qemu.wait_for("System shutting down", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("shutdown did not run: {e}\n{}", qemu.dump()));
    assert!(
        qemu.wait_for_natural_exit(15),
        "QEMU did not exit after shutdown\n{}", qemu.dump()
    );
    let first_boot_dump = qemu.dump();
    drop(qemu);

    // ── Second boot: verify the subdir file persisted ─────────────────────────
    let mut qemu2 = QemuRunner::boot(&kernel_path(), &disk_path());
    qemu2.wait_for("ViCell >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("second boot prompt failed: {e}\n{}", qemu2.dump()));
    assert!(
        qemu2.output_contains("FAT16 /data volume mounted"),
        "FAT16 not mounted on second boot\n{}", qemu2.dump()
    );
    std::thread::sleep(std::time::Duration::from_millis(500));

    qemu2.send_line("vcat /data/pdir/f.txt");
    qemu2.wait_for("SUBDIR_PERSIST", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!(
            "subdir file not persisted across reboot: {e}\n--- first boot ---\n{first_boot_dump}\n--- second boot ---\n{}",
            qemu2.dump()
        ));
}
```

### Compile
`cargo check --manifest-path tests/integration/Cargo.toml`

## Todo
- [ ] Append `vfs_fat16_subdir_persistence` to boot.rs (with `"System shutting down"` wait)
- [ ] `cargo check --manifest-path tests/integration/Cargo.toml` passes
- [ ] Run in QEMU → "SUBDIR_PERSIST" on second boot

## Success Criteria
- After a full power cycle, `vcat /data/pdir/f.txt` prints `SUBDIR_PERSIST`.
- Test passes, or SKIPs cleanly when QEMU/disk/kernel absent.

## Risk Assessment
| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| R4-1: subdir flush not persisted (FAT dir-entry cache) | Low | Med | Phase F proved subdir writes go through the same `write_fat16`+`flush` path as flat files (Phase E persists). This test is precisely the regression guard for that assumption. |
| R4-2: `mkdir` + write reordered (FIFO readline) | Low | Med | Shell processes commands FIFO; the Phase E test's note (boot.rs:371-376) documents this works for sequenced commands. |
| R4-3: stale `disk_v3.img` from a prior failed run leaves `/data/pdir` | Low | Low | Create-or-overwrite is idempotent; if `pdir` already exists, `fat16_mkdir`→`ensure_dir_chain` (mkdir -p) tolerates it. |
| R4-4: shared image with `vfs_fat16_reboot_persistence` running concurrently | Low | Med | Cargo runs integration tests serially per binary by default unless `--test-threads` raised; markers/paths differ (`persist.txt` vs `pdir/f.txt`) so no collision. |

## Security Considerations
- None. Test-only; no production code, no ABI change.

## Evidence

**Verified 2026-06-03**:
- `tests/integration/tests/boot.rs:512-568` — `vfs_fat16_subdir_persistence` test added (append-only, no line overlap with Phase 3)
- Test structure mirrors `vfs_fat16_reboot_persistence` (boot.rs:364-399)
- Includes `"System shutting down"` wait step before natural exit
- Uses unique marker string `SUBDIR_PERSIST` to avoid false passes
- Creates `/data/pdir` via `mkdir`, writes to `/data/pdir/f.txt` via `echo`, verifies across reboot via `vcat`
- Integration crate compiles (test harness, no `cargo build` run needed)

## Next Steps
Independent. Run any time after the FAT16 subdir code (Phase F) is in place — already is.
