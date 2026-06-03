# Phase 3 — QemuRunner graceful exit

## Context Links
- `tests/integration/src/lib.rs:52-57` (`QemuRunner` struct: `child: Child`, `writer: Option<TcpStream>`, `output: Arc<Mutex<String>>`)
- `tests/integration/src/lib.rs:138-145` (`send_line` uses `self.writer.as_mut()`)
- `tests/integration/src/lib.rs:158-163` (`Drop` → `child.kill()` then `child.wait()`)
- `tests/integration/src/lib.rs:12-18` (imports: `std::process::Child`, `std::time::{Duration, Instant}` already present)

## Overview
- **Priority:** P1
- **Status:** complete (2026-06-03)
- **Depends on:** nothing (independent of Phases 1 and 2)
- Adds `wait_for_natural_exit` so a test can let QEMU exit cleanly (flushing the disk) instead of SIGKILL.

## Key Insights
- `Drop` currently does `child.kill()` immediately — a SIGKILL gives the VirtIO block backend no chance to flush `disk_v3.img`. For reboot persistence we must wait for QEMU to exit on its own after the guest `shutdown`.
- `child` is `std::process::Child`; `Child::try_wait()` is non-blocking and returns `Ok(Some(status))` once exited. No new imports needed (`Duration`, `Instant` already imported at line 18; `Child` at line 15).
- `writer` is `Option<TcpStream>`. Dropping the writer (`self.writer.take()`) closes our half of the serial TCP socket so QEMU's exit isn't held by a live client. The background reader thread will see EOF and stop.
- Drop stays as the safety net: if `wait_for_natural_exit` times out (QEMU hung), `drop()` still kills. After a successful natural exit, `child.kill()` on an already-exited process is a harmless no-op / `Err` that Drop ignores.

## Requirements
- **Functional:** `wait_for_natural_exit(timeout_secs)` returns `true` iff QEMU exits within the window; leaves the process running on timeout (Drop cleans up).
- **Non-functional:** poll interval 100ms; no busy spin.

## Data flow
```
test: qemu.send_line("shutdown")
  → guest powers off → QEMU process exits, flushes disk_v3.img
test: qemu.wait_for_natural_exit(15)
  → loop { child.try_wait() }
      Ok(Some(_)) → return true   (disk flushed, safe to reboot)
      Ok(None)    → sleep 100ms, re-check until deadline
      deadline    → return false  (caller asserts; Drop kills)
```

## Related Code Files
- **Modify:** `tests/integration/src/lib.rs` (add method to `impl QemuRunner`)
- **Create / delete:** none

## Implementation Steps

### 3a. Add `wait_for_natural_exit` to `impl QemuRunner` (after `dump()`, before the `impl` close at ~line 156)
```rust
    /// Wait for QEMU to exit on its own (e.g. after a guest `shutdown`).
    ///
    /// Returns `true` if the process exited within `timeout_secs`. On timeout the
    /// process is left running and `Drop` will kill it. Used by reboot-persistence
    /// tests so the VirtIO block backend flushes the disk image before re-booting.
    ///
    /// Closes our serial writer first so QEMU's exit is not held open by a live
    /// TCP client; the background reader then sees EOF and stops.
    pub fn wait_for_natural_exit(&mut self, timeout_secs: u64) -> bool {
        // Release our half of the serial socket so QEMU can fully tear down.
        self.writer.take();

        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return true,  // exited naturally — disk flushed
                Ok(None) => {}               // still running
                Err(_) => return false,      // wait failed — let Drop handle it
            }
            if Instant::now() > deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
```
> `thread`, `Instant`, `Duration` are already imported (lines 17-18). `child` and `writer` fields exist (lines 53-54).

## Todo List
- [ ] 3a: add `wait_for_natural_exit` method
- [ ] `cargo check --manifest-path tests/integration/Cargo.toml`

## Success Criteria
- Compiles clean.
- Method returns `true` quickly when the guest shuts down; returns `false` after `timeout_secs` if QEMU hangs (exercised by Phase 4).

## Risk Assessment
| Risk | L | I | Mitigation |
|------|---|---|-----------|
| Dropping writer breaks a later `send_line` | Low | Low | Only called at end-of-life before reboot; the runner is dropped right after. `send_line` already guards `if let Some(w)`. |
| QEMU hangs on shutdown → false | Med | Med | 15s timeout in Phase 4; assert + Drop fallback kill. |
| Already-exited `child.kill()` in Drop errors | Low | Low | Drop ignores the `Result` (`let _ =`). |

## Security Considerations
- None (test harness only, never shipped).

## Evidence (Complete)
- `tests/integration/src/lib.rs:145–165` — wait_for_natural_exit() method added
- Compiles clean; all 14 integration tests pass
- Phase 4 `vfs_fat16_reboot_persistence` demonstrates graceful exit gates the disk flush

## Next Steps
- Phase 4 calls this method to gate the reboot.
