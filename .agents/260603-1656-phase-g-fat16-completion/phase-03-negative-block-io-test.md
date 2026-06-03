# Phase 3: Negative Block-I/O Test

## Context Links
- `libs/ostd/src/syscall.rs:63` — `sys_blk_read(sector: u64, buf: &mut [u8; 512]) -> bool`
- `cells/apps/shell/src/cmd_sys.rs` — system built-ins (cmd signature pattern at lines 10-56)
- `cells/apps/shell/src/executor.rs:133` — `dispatch_builtin`; match arms 143-164
- `cells/apps/shell/src/executor.rs:209` — `make_parts(args)` adapter
- `tests/integration/tests/boot.rs:13` — `BOOT_TIMEOUT=40`, `:15 CMD_TIMEOUT=10`
- `tests/integration/src/lib.rs:139,155,174,179` — `send_line`, `wait_for_natural_exit`, `output_contains`, `dump`

## Overview
- **Priority:** P2
- **Status:** pending
- **Description:** Add a `blktest` shell command that attempts a raw block read from the
  shell cell (non-VFS) and prints the result, plus an integration test asserting the
  capability gate denies it.
- **Depends on Phase 1** — the gate under test is the new `can_block_io` flag.

## Key Insights (verified 2026-06-03)
- **CORRECTION:** `sys_blk_read` takes `&mut [u8; 512]` (fixed array ref), NOT `&mut [u8]`.
  So `let mut buf = [0u8; 512]; sys_blk_read(0, &mut buf)` type-checks (`&mut [0u8;512]`).
- It returns `bool`: `true` only if kernel returned `1`. On `PermissionDenied` the kernel
  sets the error path → `ret != 1` → `false`. So `if sys_blk_read(...) { ALLOWED } else { denied }`.
- **CORRECTION:** Shell built-ins take `core::str::SplitWhitespace<'a>` (see cmd_sys.rs:10),
  dispatched via `make_parts(args)`. The brief's `cmd_blkio_test()` (no args) won't fit the
  dispatch shape. Use the standard `<'a>(_args: SplitWhitespace<'a>) -> ViResult<()>` signature.
- Shell is the calling cell. Its task ID is whatever the scheduler assigned (the "task 8"
  in the brief is illustrative, not load-bearing — the gate keys on `can_block_io`, not ID).
- ostd prelude: `cmd_sys.rs` already does `use ostd::prelude::*;` (gives `ViResult`) and
  `use ostd::syscall;`. `ostd::io::println` is used elsewhere in the file.

## Data Flow
```
shell `blktest` ─▶ dispatch_builtin("blktest") ─▶ cmd_blkio_test(_args)
                                                       │
                              sys_blk_read(0, &mut [0u8;512])  (ostd syscall 500)
                                                       │
                              kernel handle_syscall(caller=shell_tid, BlkRead)
                                                       │
                              caller_has_block_io(shell_tid) → false  (Phase 1)
                                                       │
                              Err(PermissionDenied) ─▶ ret != 1 ─▶ false
                                                       │
                              prints "blkio: denied" ─▶ integration test wait_for match
```

## Related Code Files
**Modify:**
- `cells/apps/shell/src/cmd_sys.rs` — add `cmd_blkio_test`
- `cells/apps/shell/src/executor.rs` — register `"blktest"` arm
- `tests/integration/tests/boot.rs` — add `block_io_denied_non_vfs` test

**Create/Delete:** none.

## Implementation Steps

### 3a. Add `cmd_blkio_test` (cmd_sys.rs)
Append (matching the file's existing signature/doc style):
```rust
/// `blktest` — attempt a raw block read from the shell cell (a non-VFS cell).
/// Prints "blkio: denied" when Phase G's capability gate correctly rejects the
/// call, or "blkio: ALLOWED (BUG)" if the gate is missing. Used by the
/// `block_io_denied_non_vfs` integration test.
pub fn cmd_blkio_test<'a>(_args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
    let mut buf = [0u8; 512];
    if syscall::sys_blk_read(0, &mut buf) {
        ostd::io::println("blkio: ALLOWED (BUG)");
    } else {
        ostd::io::println("blkio: denied");
    }
    Ok(())
}
```

### 3b. Register the built-in (executor.rs)
Add to the `dispatch_builtin` match (alongside the other `cmd_sys::*` arms, ~line 159):
```rust
        "blktest" => crate::cmd_sys::cmd_blkio_test(make_parts(args)),
```

### 3c. Add the integration test (boot.rs)
Append a new `#[test]` fn (after the FAT16 tests, append-only — no overlap with Phase 4):
```rust
/// Phase G: a non-VFS cell must NOT reach raw block I/O (capability gate).
/// The shell cell lacks `can_block_io`, so `sys_blk_read` must return false.
#[test]
fn block_io_denied_non_vfs() {
    if !prerequisites_ok() {
        return;
    }
    let mut qemu = QemuRunner::boot(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt not reached: {e}\n{}", qemu.dump()));
    std::thread::sleep(std::time::Duration::from_millis(500));

    qemu.send_line("blktest");
    qemu.wait_for("blkio: denied", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("block I/O was NOT denied for non-VFS cell: {e}\n{}", qemu.dump()));

    // Guard against a false pass: the BUG marker must never appear.
    assert!(
        !qemu.output_contains("blkio: ALLOWED"),
        "capability gate let a non-VFS cell read the block device\n{}", qemu.dump()
    );
}
```

### 3d. Compile
- `cargo check -p app-shell`
- `cargo check --manifest-path tests/integration/Cargo.toml`

## Todo
- [ ] 3a: add `cmd_blkio_test` to cmd_sys.rs (correct `<'a>(SplitWhitespace)` signature)
- [ ] 3b: register `"blktest"` arm in executor.rs via `make_parts(args)`
- [ ] 3c: add `block_io_denied_non_vfs` test to boot.rs
- [ ] 3d: `cargo check -p app-shell` + integration crate pass
- [ ] (after Phase 1) run test in QEMU → "blkio: denied"

## Success Criteria
- `blktest` from the shell prints `blkio: denied`.
- Test passes (or SKIPs cleanly without QEMU/disk/kernel).
- `blkio: ALLOWED` never appears in output (false-pass guard).

## Risk Assessment
| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| R3-1: test run before Phase 1 → gate is still `VFS_TASK_ID` | Med (ordering) | Med | Plan mandates 1→3 apply order; both deny shell anyway (shell tid ≠ 3), so test still passes — but the assertion it documents is the new flag. |
| R3-2: `sys_blk_read` signature mismatch (`&mut [u8]` vs `&mut [u8;512]`) | Low | Low | Verified: fixed-array ref; `&mut [0u8;512]` compiles. |
| R3-3: shell built-in signature mismatch breaks dispatch | Low | Low | Use the verified `<'a>(_args: SplitWhitespace<'a>) -> ViResult<()>` + `make_parts(args)`. |
| R3-4: `wait_for` matches a stale earlier line | Low | Low | `blkio: denied` is a unique never-before-printed string. |

## Security Considerations
- This is a security regression test: it locks in that block-I/O is VFS-only. If a future
  change accidentally grants the flag too broadly, this test fails loudly.

## Evidence

**Verified 2026-06-03**:
- `cells/apps/shell/src/cmd_sys.rs:72-81` — `cmd_blkio_test()` added with correct signature `(SplitWhitespace<'a>) -> ViResult<()>`
- `cells/apps/shell/src/executor.rs` — `"blktest"` arm registered in `dispatch_builtin` match
- `tests/integration/tests/boot.rs:486-510` — `block_io_denied_non_vfs` test added, asserts output contains "blkio: denied" and NOT "blkio: ALLOWED"
- `cargo build -p app-shell -r` passes (1 dead_code warning is pre-existing)

## Next Steps
Run after Phase 1 lands. Independent of Phases 2 and 4.
