---
phase: 4
title: "Integration test (echo > file > cat)"
status: complete
priority: P2
effort: 1h
dependencies: [1, 2, 3]
completed: 2026-06-03
---

# Phase 4: Integration Test — VFS Write Round-Trip

## Context Links
- `tests/integration/tests/boot.rs:1-54` — test harness helpers (repo_root, kernel_path, disk_path, prerequisites_ok)
- `tests/integration/tests/boot.rs:58-65` — `boots_to_shell_prompt` (template for shell-prompt tests)
- `tests/integration/src/lib.rs:65` — `QemuRunner::boot(kernel, disk) -> Self`
- `tests/integration/src/lib.rs:122` — `wait_for(&self, pattern, timeout_secs) -> Result<String,String>`
- `tests/integration/src/lib.rs:139` — `send_line(&mut self, line)` — **requires `mut` binding**
- `tests/integration/src/lib.rs:153` — `dump(&self) -> String`

## Overview
- **Priority:** P2
- **Status:** pending
- **Description:** End-to-end test: boot to shell, `echo PHASE_C_WRITE > /tmp/test.txt`, then
  `cat /tmp/test.txt` and assert the written bytes are read back. Validates Phases 1-3 together.

## Key Insights
- VERIFIED harness API: `boot`, `wait_for`, `send_line`, `dump`, `kernel_path`, `disk_path`,
  `prerequisites_ok`, `BOOT_TIMEOUT=40` all exist. The proposed test compiles against them.
- CORRECTION: `send_line` takes `&mut self` (lib.rs:139); the existing `boots_to_shell_prompt`
  binds `let qemu` (immutable) because it never sends. Our test MUST use `let mut qemu`.
- CORRECTION: research's test used `BOOT_TIMEOUT` for `wait_for("ViOS >", BOOT_TIMEOUT)` then small
  literals (5) for follow-ups. That is fine, but for portability reuse `BOOT_TIMEOUT` for the first
  prompt and a small `CMD_TIMEOUT` const for command round-trips.
- `prerequisites_ok()` returns (skips) cleanly when QEMU/kernel/disk missing — keep that guard.
- The `cat` built-in reads via kernel FS (`sys_open`/`sys_read`), NOT via VFS OP_GET_FILE
  (see commands.rs cmd_cat / cmd_fs read_file_bytes uses sys_open). VERIFY that a file written into
  the VFS-cell RamFS is visible to `cat`'s kernel-FS read path. THIS IS THE KEY INTEGRATION RISK —
  see Risk table. If `cat` cannot see RamFS writes, the test must read back via a path that does
  (e.g. a `cat`-equivalent that uses OP_GET_FILE), or Phase 3 must also expose a VFS-backed read.

## Requirements
**Functional**
- Test boots, sends the echo-redirect line, waits for prompt return, sends `cat`, asserts content.
- Test SKIPS (not fails) when prerequisites missing.

**Non-functional**
- No new harness API; uses existing `QemuRunner`.
- Deterministic: explicit `wait_for` between sends; short sleep after first prompt (serial settle).

## Architecture
**Test flow:**
```
boot → wait_for("ViOS >", 40)
     → send_line("echo PHASE_C_WRITE > /tmp/test.txt")
     → wait_for("ViOS >", CMD_TIMEOUT)        (prompt returns = command done)
     → send_line("cat /tmp/test.txt")
     → wait_for("PHASE_C_WRITE", CMD_TIMEOUT) (assert content read back)
```

## Related Code Files
**Modify**
- `tests/integration/tests/boot.rs` — append `vfs_write_echo_redirect` test + optional
  `CMD_TIMEOUT` const.

**Create / Delete** — none.

## Implementation Steps
1. Add near other consts: `const CMD_TIMEOUT: u64 = 10;`
2. Append the test (note `let mut qemu`):
   ```rust
   /// Phase C: VFS write — echo redirected to /tmp, then cat reads it back.
   #[test]
   fn vfs_write_echo_redirect() {
       if !prerequisites_ok() {
           return;
       }
       let mut qemu = QemuRunner::boot(&kernel_path(), &disk_path());
       qemu.wait_for("ViOS >", BOOT_TIMEOUT)
           .unwrap_or_else(|e| panic!("shell not reached: {e}\n{}", qemu.dump()));
       // Let the serial line settle before injecting input.
       std::thread::sleep(std::time::Duration::from_millis(500));

       qemu.send_line("echo PHASE_C_WRITE > /tmp/test.txt");
       qemu.wait_for("ViOS >", CMD_TIMEOUT)
           .unwrap_or_else(|e| panic!("prompt not returned after write: {e}\n{}", qemu.dump()));

       qemu.send_line("cat /tmp/test.txt");
       qemu.wait_for("PHASE_C_WRITE", CMD_TIMEOUT)
           .unwrap_or_else(|e| panic!("file content not read back: {e}\n{}", qemu.dump()));
   }
   ```
3. `cargo check --manifest-path tests/integration/Cargo.toml`
4. Build prerequisites then run:
   ```
   cargo build --release -p vios-kernel
   ./gen_disk.ps1   # if disk_v3.img stale
   cargo test --manifest-path tests/integration/Cargo.toml vfs_write_echo_redirect -- --nocapture
   ```

## Todo List
- [ ] Add `CMD_TIMEOUT` const
- [ ] Add `vfs_write_echo_redirect` test with `let mut qemu`
- [ ] `cargo check` the integration crate
- [ ] Build kernel + disk, run the test, confirm pass
- [ ] If `cat` can't see RamFS write, resolve read-back path (see Risk)

## Success Criteria
- Test compiles.
- With QEMU + built kernel + disk present, test passes: `PHASE_C_WRITE` appears after `cat`.
- Without prerequisites, test prints SKIP and returns green.

## Risk Assessment
| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| `cat` reads via kernel-FS, not the VFS-cell RamFS → can't see OP_WRITE result | HIGH | HIGH | THE central integration risk. Trace cat's read path (commands.rs cmd_cat → sys_open). If kernel FS and VFS-cell RamFS are separate stores, the round-trip fails. Mitigation options: (a) verify whether kernel `sys_open` for `/tmp/*` routes to the VFS cell; (b) if not, add a `cat`-via-OP_GET_FILE built-in OR assert by re-reading through VFS in shell; (c) worst case, narrow Phase 4 to assert the write succeeded (no error line) + a shell-side read-back command added in Phase 3. RESOLVE before writing test. |
| QEMU serial timing flakiness | Medium | Low | 500ms settle + explicit wait_for between sends; CMD_TIMEOUT=10s generous. |
| Prompt string differs ("ViOS >" vs "ViOS > ") | Low | Low | Reuse exact literal from boots_to_shell_prompt (boot.rs:64). |
| disk_v3.img stale/missing | Low | Low | prerequisites_ok skips; doc says run ./gen_disk.ps1. |

## Security Considerations
- Test-only; no production surface. Asserts the /tmp guard indirectly (write to /tmp succeeds).
- Optionally add a negative assertion: `echo X > /etc/p` should NOT round-trip. Deferred to keep
  the test focused; note as a follow-up.

## Evidence (Phase Complete)
- `tests/integration/tests/boot.rs` added:
  - `const CMD_TIMEOUT: u64 = 10;` — command round-trip timeout (distinct from BOOT_TIMEOUT)
  - `#[test] fn vfs_write_echo_redirect()` test function with `let mut qemu` binding
  - Test flow: boot → wait for prompt → send `echo PHASE_C_WRITE > /tmp/test.txt` → wait for prompt → send `vcat /tmp/test.txt` → assert `PHASE_C_WRITE` in output
  - Uses `vcat` built-in (added Phase 3) to read back via VFS OP_READ (same cell store as OP_WRITE)
- Integration test result: **12/12 tests pass** ✅ (including vfs_write_echo_redirect)
- Manual verification in QEMU:
  - `echo PHASE_C_WRITE > /tmp/test.txt` writes silently ✅
  - `vcat /tmp/test.txt` reads back `PHASE_C_WRITE` ✅
  - `echo X > /etc/passwd` rejected by /tmp guard, prints error ✅

**Design Note:** Test uses `vcat` (VFS-backed read via OP_READ) rather than `cat` (kernel-FS read) because the kernel FS and VFS-cell RamFS are separate backing stores. `cat` cannot see writes to the VFS-cell store. Phase C does not integrate kernel FS with VFS RamFS (Phase D scope). The `vcat` read-back proves the round-trip within the VFS cell, which is the success criterion for Phase C.

## Unresolved Questions
- None. Phase C round-trip complete via VFS cell store (write OP_WRITE → read OP_READ).

## Next Steps
- Final phase. On pass, update `docs/project-changelog.md` and `docs/development-roadmap.md`
  (Phase C complete, RamFS write volatile, Phase D = FAT32 persistence).
