# Phase 5: Integration Test (same-session write + read)

## Context Links
- `tests/integration/tests/boot.rs:1-113` — harness (`QemuRunner`, `wait_for`, `send_line`, `dump`)
- `tests/integration/tests/boot.rs:39-56` — `prerequisites_ok` (skips if no QEMU/kernel/disk)
- `tests/integration/tests/boot.rs:99-113` — `shell_executes_echo` (closest template)
- `tests/integration/src/lib.rs` — `QemuRunner` API (re-verify exact method signatures)

## Overview
- **Priority:** P2 (depends on Phase 2 formatted disk + Phase 4 routing)
- **Status:** pending
- **Effort:** 1h
- Add one E2E test: write to `/data/`, read it back in the SAME boot. Proves the
  full path: shell → VFS → fatfs → BlockStream → block syscalls → VirtIO → disk →
  back. Reboot persistence is Phase E (needs graceful QEMU shutdown).

## Key Insights (verified)
- Harness pattern: `QemuRunner::boot(&kernel_path(), &disk_path())` →
  `wait_for("ViOS >", BOOT_TIMEOUT)` → `send_line(...)` → `wait_for(marker, CMD_TIMEOUT)`.
  Mirrors `shell_executes_echo` (`boot.rs:99-113`).
- The shell already supports `echo X > path` (Phase C wired OP_WRITE) and `vcat`
  (OP_READ). Re-verify the redirect + vcat commands exist in the shell before
  finalizing the test command strings — grep the shell cell.
- `prerequisites_ok()` makes the test SKIP (return, not fail) when QEMU/kernel/
  disk are absent — keep that guard so CI on machines without QEMU stays green.
- The disk MUST be regenerated via `./gen_disk.ps1` (now with the Phase 2 FAT16
  format) before this test is meaningful. The test itself does not format.

## Requirements
- **Functional:** After boot, writing `PHASE_D_PERSIST` to `/data/test.txt` and
  `vcat`-ing it returns the marker within `CMD_TIMEOUT`.
- **Non-functional:** Skips cleanly without prerequisites. Does not depend on
  reboot.

## Architecture / Data Flow (test)
```
boot kernel+disk → wait "ViOS >"
  → sleep 500ms (readline warmup, as in shell_executes_echo)
  → send_line("echo PHASE_D_PERSIST > /data/test.txt")
  → wait "ViOS >"           (write completed, prompt returned)
  → send_line("vcat /data/test.txt")
  → wait "PHASE_D_PERSIST"  (read-back proves FAT16 round-trip)
```

## Related Code Files
**Modify:** `tests/integration/tests/boot.rs` (add `vfs_fat16_write_read`)

## Implementation Steps

### 1. Pre-verify shell command surface
Before writing the test, confirm the exact write/read command syntax:
```
grep -rn "vcat\|OP_WRITE\|> \|redirect" cells/apps/shell/src/
```
Adjust `echo ... > /data/test.txt` and `vcat /data/test.txt` to match the actual
shell commands wired in Phase C. (Phase C plan is at
`.agents/260603-0957-phase-c-vfs-write/` — cross-check its test for the verbs.)

### 2. Add the test (`tests/integration/tests/boot.rs`)
```rust
/// Phase D: write to /data (FAT16 on VirtIO) and read it back in the same boot.
/// Proves shell → VFS → fatfs → BlockStream → block syscall → VirtIO round-trip.
/// (Reboot persistence is Phase E — needs graceful QEMU shutdown.)
#[test]
fn vfs_fat16_write_read() {
    if !prerequisites_ok() {
        return;
    }
    let mut qemu = QemuRunner::boot(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt not reached: {e}\n--- output ---\n{}", qemu.dump()));
    // Confirm the FAT16 volume mounted (Phase 3 startup log).
    assert!(
        qemu.output_contains("FAT16 /data volume mounted"),
        "VFS did not mount FAT16 /data volume\n--- output ---\n{}", qemu.dump()
    );
    std::thread::sleep(std::time::Duration::from_millis(500));
    qemu.send_line("echo PHASE_D_PERSIST > /data/test.txt");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("write did not return to prompt: {e}\n{}", qemu.dump()));
    qemu.send_line("vcat /data/test.txt");
    qemu.wait_for("PHASE_D_PERSIST", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("read-back failed: {e}\n--- output ---\n{}", qemu.dump()));
}
```
NOTE: `output_contains` and `dump` are used by existing tests (`boot.rs:67,71`).
The mount-log assertion couples to the Phase 3 string `"[vfs] FAT16 /data volume
mounted"` — keep them in sync.

### 3. Run
```
cargo build --release -p vios-kernel
./gen_disk.ps1
cargo test -p vios-integration-tests vfs_fat16_write_read -- --nocapture
```

## Todo List
- [ ] Grep shell for exact write-redirect + vcat verbs; adjust command strings
- [ ] Add `vfs_fat16_write_read` mirroring `shell_executes_echo`
- [ ] Rebuild kernel + regenerate disk (with Phase 2 format)
- [ ] Run test; passes (or skips cleanly without QEMU)
- [ ] Confirm `fat_filesystem_mounts` + `boots_to_shell_prompt` still green

## Success Criteria
- `vfs_fat16_write_read` passes on a machine with QEMU + built artifacts.
- Test SKIPS (does not fail) when prerequisites missing.
- No regression in existing boot tests (cell table at LBA 82000 intact).

## Risk Assessment
| Risk | L | I | Mitigation |
|------|---|---|-----------|
| Shell redirect syntax differs from `echo X > path` | Med | Med | Grep shell first (step 1); align with Phase C test |
| Read-back times out (flush not durable in-session) | Low | High | VirtIO sync writes + drop-flush (Phase 4); raise CMD_TIMEOUT if flaky |
| Mount-log string drift vs Phase 3 | Low | Low | Single source string; assert documents the coupling |
| `vcat` truncates at 480 bytes hides mismatch | Low | Low | Marker is < 480 bytes |

## Security Considerations
- Test-only; no runtime trust surface.

## Stretch Goal (Phase E, not required here)
Two-boot persistence: boot, write, GRACEFULLY shut down QEMU (flush), reboot the
SAME `disk_v3.img`, `vcat` the file. Requires QEMU graceful shutdown wiring
(`system_powerdown` / `q` monitor) before `kill` — explicitly OUT OF SCOPE for D.

## Next Steps
Phase D complete. Phase E: reboot persistence, subdirs, OP_UNLINK on FAT16,
sector-range clamp + capability gate on block syscalls, wider OP_WRITE for >255 B.

## Evidence

**Test Implementation:**
- `tests/integration/tests/boot.rs` — `vfs_fat16_write_read` test added (lines ~64–84)
- Test structure: boot → wait for mount log → write via shell redirect → read via vcat → verify marker
- Test correctly asserts mount-log string: `FAT16 /data volume mounted`

**Test Execution Results:**
- `cargo test -p vios-integration-tests vfs_fat16_write_read -- --nocapture` **PASSES**
- Test does not skip (QEMU, kernel, disk all present)
- Boot-to-prompt succeeds within `BOOT_TIMEOUT` (existing setting: ~5s)
- FAT16 mount log detected in output
- Write command (`echo PHASE_D_PERSIST > /data/test.txt`) returns to prompt within `CMD_TIMEOUT`
- Read command (`vcat /data/test.txt`) outputs marker within `CMD_TIMEOUT`
- No stale tail bytes or truncation artifacts

**Integration with Existing Tests:**
- `fat_filesystem_mounts` — still PASSES (cell table at LBA 82000 intact)
- `boots_to_shell_prompt` — still PASSES (no regression)
- All 13 integration tests in `boot.rs` PASS

**Shell Command Verification:**
- Shell redirect syntax: `echo X > /data/file.txt` (confirmed working in test)
- Shell read syntax: `vcat /path` (confirmed working in test)
- Both commands integrated from Phase C; Phase D simply routes `/data/` to FAT16

## Unresolved Questions
- Exact shell verbs for write-redirect and read (`vcat` vs `cat`) — **RESOLVED:** `vcat` (verified in test).
