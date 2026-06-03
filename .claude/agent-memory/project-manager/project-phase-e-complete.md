---
name: project-phase-e-complete
description: Phase E (Hardening + Reboot Persistence) completion 2026-06-03; 14/14 tests pass; FAT16 persistence proven
metadata:
  type: project
---

## Phase E: Hardening + Reboot Persistence — COMPLETE 2026-06-03

**Plan:** `d:/ViCell/.agents/260603-1105-phase-e-reboot-persistence/`

All 4 sub-phases complete; all 14 integration tests pass.

### Phase 1 — Hardening
- **Files modified:** `cells/services/vfs/src/block_stream.rs`, `kernel/src/task/syscall.rs`
- **Changes:** SeekFrom::Current underflow guard (validates result ≥ 0 before cast); BlkRead/BlkWrite sector cap at CELL_TABLE_BASE_LBA
- **Rationale:** Closes two Phase D code-review findings; prevents cell from corrupting kernel bootstrap table

### Phase 2 — Shutdown syscall
- **Files modified:** `kernel/src/task/syscall.rs`, `libs/ostd/src/syscall.rs`, `cells/apps/shell/src/cmd_sys.rs`, `cells/apps/shell/src/executor.rs`
- **Changes:** Syscall 502 (raw, no ViSyscall enum entry); SBI SRST handler; `sys_shutdown() -> !`; `shutdown` built-in command
- **Rationale:** Enables graceful system powerdown (required for disk flush before reboot test)

### Phase 3 — QemuRunner graceful exit
- **Files modified:** `tests/integration/src/lib.rs`
- **Changes:** Added `wait_for_natural_exit(timeout_secs)` method to poll `child.try_wait()` instead of immediate SIGKILL
- **Rationale:** Allows VirtIO block backend to flush disk_v3.img before second boot reads it

### Phase 4 — Reboot persistence test
- **Files modified:** `tests/integration/tests/boot.rs`
- **Changes:** Added `vfs_fat16_reboot_persistence` test (write REBOOT_OK → shutdown → reboot → read-back)
- **Rationale:** Proves FAT16 write durability across power cycle

### Critical Bug Fixed During Phase
- **File:** `cells/apps/shell/src/shell.rs`
- **Root cause:** Pre-parser echo handler at `dispatch()` that split by whitespace, completely bypassing redirect parser
- **Impact:** All `echo X > /path` commands printed to console instead of writing to file
- **Fix:** Removed pre-parser echo handler; echo now goes through normal parser which correctly handles RedirectOut syntax
- **Verification:** Phase 4 integration test proves echo redirection working (write persists across reboot)

### Evidence
- `cargo test -p vios-integration-tests` — all 14 tests pass
- `cargo check` clean across kernel, vfs, ostd, shell, integration targets
- `shutdown` built-in cleanly terminates QEMU within 15s
- Phase 4 test reads back `REBOOT_OK` marker from FAT16 after reboot — persistence proven

### Impact
- Closes 2 Phase D safety findings
- Enables clean shutdown path (prerequisite for graceful disk sync)
- Proves filesystem persistence across power cycle (critical for real OS)
- Fixes shell echo-redirect bug (enables `>` redirection in scripts/commands)
- Unblocks Phase F features (ACPI/PSCI power mgmt, power-loss recovery, stress testing)

### Documentation updated
- `docs/system-architecture.md` — added Phase E section under VirtIO
- `docs/project-changelog.md` — added Phase E entry with full change list
- All plan.md and phase-*.md files marked complete with Evidence sections
