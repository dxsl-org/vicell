# Phase 4 — Reboot persistence integration test

## Context Links
- `tests/integration/tests/boot.rs:13-15` (`BOOT_TIMEOUT = 40`, `CMD_TIMEOUT = 10`, verified)
- `tests/integration/tests/boot.rs:26-41` (`kernel_path`, `disk_path`, `prerequisites_ok`, verified)
- `tests/integration/tests/boot.rs:329-354` (`vfs_fat16_write_read` — the same-session template)
- `cells/services/vfs/src/main.rs:300` (emits `[vfs] FAT16 /data volume mounted`, verified)
- `cells/services/vfs/src/main.rs:243-260` (`write_fat16` → `/data/NAME` create-or-overwrite)
- `cells/apps/shell/src/executor.rs:88-100` (`echo > path` → `write_file` → FAT16, verified)
- Phase 2 (`shutdown` built-in) and Phase 3 (`wait_for_natural_exit`) — **hard dependencies**

## Overview
- **Priority:** P1
- **Status:** complete (2026-06-03)
- **Depends on:** Phase 2 (shutdown built-in), Phase 3 (`wait_for_natural_exit`)
- Two-boot test: write marker to `/data/`, shut down, wait for clean exit, reboot from the same `disk_v3.img`, read the marker back.

## Key Insights
- This is the only test that proves persistence across a process boundary; `vfs_fat16_write_read` only proves same-session round-trip.
- The write mechanism is already wired: `echo REBOOT_OK > /data/persist.txt` → parser `RedirectOut` → `executor.rs:88` echo-capture → `cmd_fs::write_file` → OP_WRITE → `write_fat16` (`main.rs:243`). Verified end-to-end; no new guest code needed beyond Phase 2.
- The clean-exit gate (Phase 3) is essential: a SIGKILL'd QEMU may not flush the raw `disk_v3.img` backend, so the second boot could read stale data.
- The mount banner substring to assert is `FAT16 /data volume mounted` (full line `[vfs] FAT16 /data volume mounted`, `main.rs:300`). Use `output_contains`, not `wait_for`, since it appears during boot before the prompt.
- Constants: `BOOT_TIMEOUT = 40`, `CMD_TIMEOUT = 10` (verified `boot.rs:13-15`). The shutdown wait uses a literal `15` (independent of boot/cmd timeouts).

## Requirements
- **Functional:** after reboot, `vcat /data/persist.txt` outputs `REBOOT_OK`.
- **Non-functional:** test skips (not fails) when `prerequisites_ok()` is false (no QEMU / no built kernel / no disk) — matches every other test in the file.

## Data flow / test matrix
```
Boot #1 (QemuRunner A, disk_v3.img)
  wait_for "ViOS >"              (boot reached shell)
  assert output_contains "FAT16 /data volume mounted"   (mount OK on boot 1)
  send_line "echo REBOOT_OK > /data/persist.txt"
  wait_for "ViOS >"             (write returned to prompt → OP_WRITE durable)
  send_line "shutdown"          (Phase 2 built-in → SBI SRST)
  wait_for_natural_exit(15)     (Phase 3 → QEMU exits, disk flushed)  ── GATE
  drop(A)                       (safe; process already gone)

Boot #2 (QemuRunner B, SAME disk_v3.img)
  wait_for "ViOS >"
  assert output_contains "FAT16 /data volume mounted"   (mount OK on boot 2)
  send_line "vcat /data/persist.txt"
  wait_for "REBOOT_OK"          ── PASS criterion (persistence proven)
```

| Layer | What is validated |
|-------|-------------------|
| Unit | (covered by Phase 1/2/3 cargo check) |
| Integration (this) | FAT16 write + clean shutdown + reboot + read-back |
| End-to-end | Full boot → shell → FS → power-off → reboot chain |

## Related Code Files
- **Modify:** `tests/integration/tests/boot.rs` (append one `#[test]` fn)
- **Create / delete:** none
- **Disk artifact:** uses existing `disk_v3.img` (NOT regenerated — that would erase the marker between boots).

## Implementation Steps

### 4a. Append the test to `boot.rs` (after `vfs_fat16_write_read`, ~line 354)
```rust
/// Phase E: a FAT16 write survives a full reboot.
///
/// Boots QEMU, writes a marker to `/data/`, issues the `shutdown` built-in,
/// waits for QEMU to exit cleanly (flushing the VirtIO-backed disk image), then
/// boots a SECOND QEMU instance against the same `disk_v3.img` to verify the
/// marker persisted across the power cycle.
#[test]
fn vfs_fat16_reboot_persistence() {
    if !prerequisites_ok() {
        return;
    }

    // ── First boot: write the marker ─────────────────────────────────────
    let mut qemu = QemuRunner::boot(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("first boot prompt failed: {e}\n{}", qemu.dump()));
    assert!(
        qemu.output_contains("FAT16 /data volume mounted"),
        "FAT16 not mounted on first boot\n{}", qemu.dump()
    );

    std::thread::sleep(std::time::Duration::from_millis(500));
    qemu.send_line("echo REBOOT_OK > /data/persist.txt");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("write did not return to prompt: {e}\n{}", qemu.dump()));

    // ── Graceful shutdown — lets VirtIO flush disk_v3.img ─────────────────
    qemu.send_line("shutdown");
    assert!(
        qemu.wait_for_natural_exit(15),
        "QEMU did not exit after shutdown command\n{}", qemu.dump()
    );
    drop(qemu); // safe: process already exited; Drop's kill is a no-op

    // ── Second boot: verify persistence ──────────────────────────────────
    let mut qemu2 = QemuRunner::boot(&kernel_path(), &disk_path());
    qemu2.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("second boot prompt failed: {e}\n{}", qemu2.dump()));
    assert!(
        qemu2.output_contains("FAT16 /data volume mounted"),
        "FAT16 not mounted on second boot\n{}", qemu2.dump()
    );

    std::thread::sleep(std::time::Duration::from_millis(500));
    qemu2.send_line("vcat /data/persist.txt");
    qemu2.wait_for("REBOOT_OK", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("persistence failed: {e}\n--- output ---\n{}", qemu2.dump()));
}
```
> `dump()` borrows `&self`; calling it inside the `qemu.wait_for(...)` panic closure after `qemu` was used by `wait_for` (also `&self`) is fine — no move occurs. `send_line`/`wait_for_natural_exit` take `&mut self`, so `qemu` must be `let mut` (it is).

## Todo List
- [ ] 4a: add `vfs_fat16_reboot_persistence` test
- [ ] `cargo check --manifest-path tests/integration/Cargo.toml`
- [ ] Build kernel release: `cargo build --release` (so `kernel_path()` exists)
- [ ] Run: `cargo test -p vios-integration-tests vfs_fat16_reboot_persistence -- --nocapture`
- [ ] Confirm `vfs_fat16_write_read` still passes (no regression)

## Success Criteria
- Test passes: second boot's `vcat` emits `REBOOT_OK`.
- If QEMU/kernel/disk absent, test returns early (skips) — never a false failure.

## Risk Assessment
| Risk | L | I | Mitigation |
|------|---|---|-----------|
| SBI SRST unsupported → `shutdown` hangs → `wait_for_natural_exit` false | Med | High | Assert message surfaces it; fallback = SBI legacy shutdown from S-mode (Phase 2 note). |
| QEMU raw-disk backend doesn't flush on clean exit | Low | High | Default file backend fsyncs on graceful exit; if flaky, add `cache=writethrough` to the `-drive` line in `lib.rs`. |
| Second QEMU reads stale host page cache | Low | Med | Disk is a file; host page cache is coherent across sequential processes (boot #1 fully exits before boot #2 opens it). |
| 500ms sleeps too short for async readline | Low | Low | Matches the 500ms used by passing tests (`boot.rs:130,150,346`). |
| Marker pollutes `disk_v3.img` for later runs | Low | Low | Idempotent: `write_fat16` is create-or-overwrite; re-runs simply rewrite `REBOOT_OK`. |

## Security Considerations
- None (test harness only).

## Evidence (Complete)
- `tests/integration/tests/boot.rs:362–409` — vfs_fat16_reboot_persistence() test added
- All 14 integration tests pass, including new reboot-persistence test
- Second-boot `vcat` outputs `REBOOT_OK` confirming OP_WRITE durability across reboot
- No regressions: `vfs_fat16_write_read` still passes

## Bug Fix Applied During Phase
- **Root cause:** `cells/apps/shell/src/shell.rs` had a pre-parser echo-interception handler at `dispatch()` that split by whitespace, completely bypassing the redirect parser
- **Impact:** All `echo X > /path` commands printed to console instead of writing to file
- **Fix:** Removed pre-parser echo handler; echo now goes through normal parser which correctly handles RedirectOut syntax
- **Verification:** Phase 4 test proves echo redirection working (write persists across reboot)

## Next Steps
- After green: update `docs/development-roadmap.md` (Phase E complete) and `docs/project-changelog.md` (FAT16 reboot persistence + two hardening fixes).
