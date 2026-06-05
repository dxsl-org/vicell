# Phase 05 — Integration Test Suite

**Status**: 📋 PLANNED  
**Priority**: P1  
**Effort**: 3 days  
**Depends on**: Phases 01-04 (tests validate their output)

---

## Context Links

- Existing manual test helpers: `cells/apps/shell/src/cmd_fs.rs:196-358` — `vwrite`, `vappend`, `vcat`, `vwrite`
- Test isolation cell: `cells/apps/test-isolation/` (exists, not VFS-specific)
- Integration test pattern: existing tests are manual shell commands; goal is automated assertions
- VFS IPC: `libs/api/src/ipc.rs` — `VfsRequest`/`VfsResponse`

---

## Overview

Zero automated integration tests exist for VFS operations. All current testing is manual (run shell commands, check output by eye). This phase creates a formal test cell that exercises the VFS service programmatically and asserts expected responses.

---

## Test Cell Design

Create `cells/apps/vfs-test/` — a dedicated test binary that:
1. Sends VfsRequest messages directly to the VFS endpoint
2. Asserts expected responses
3. Logs PASS/FAIL for each scenario
4. Calls `sys_exit(0)` on all pass, `sys_exit(1)` on any failure

The test cell is spawnable from the shell: `spawn /bin/vfs-test`.

---

## Test Scenarios

### 1. File lifecycle round-trip

```
write("/data/test.txt", "hello world")    → Ok
get("/data/test.txt")                      → DataPtr { len: 11 }
verify: content == "hello world"
unlink("/data/test.txt")                   → Ok
get("/data/test.txt")                      → Err(1) (not found)
```

### 2. Directory operations

```
mkdir("/data/testdir")                     → Ok
mkdir("/data/testdir/subdir")              → Ok
write("/data/testdir/file.txt", "x")       → Ok
listdir("/data/testdir")                   → contains "f:file.txt\n", "d:subdir\n"
listdir("/data/testdir/subdir")            → empty (no entries)
rmdir("/data/testdir/subdir")              → Ok
listdir("/data/testdir")                   → does NOT contain "d:subdir\n"
```

### 3. Quota enforcement (Phase 01)

```
// Write until quota exceeded
for i in 0..33:
    write("/data/quota_chunk_{i}", 1MB_buffer)
// 33rd write should fail
assert: 33rd response == Err(2) (quota exceeded)
// Cleanup
unlink each chunk → Ok
// Verify quota released
write("/data/after_release", "small")     → Ok
```

### 4. Access control (Phase 03)

```
// /bin/ is read-only
write("/bin/evil", "hack")                → Err(3) (PermissionDenied)
// /data/ is writable
write("/data/ok.txt", "allowed")          → Ok
```

### 5. Async read (Phase 04)

```
// Pre-create a test file
write("/data/async_test.txt", "async content")
// Async read
ReadAsync("/data/async_test.txt")         → PendingHandle(n)
Poll(n)                                   → Data("async content")
// Stale handle
Poll(n)                                   → Err (handle consumed)
```

### 6. RamFS operations

```
write("/tmp/volatile.txt", "volatile")    → Ok
get("/tmp/volatile.txt")                  → DataPtr (correct content)
stat("/tmp/volatile.txt")                 → Stat { size: 8, is_dir: false }
stat("/tmp")                              → Stat { size: 0, is_dir: true }
```

### 7. Edge cases

```
write("relative/path", "x")              → Err (no leading /)
write("", "x")                            → Err (empty path)
get("/data/does_not_exist")              → Err(1)
mkdir("/data/")                           → Err (trailing slash, or Ok idempotent)
listdir("/data/nonexistent_dir")         → empty or Err
```

---

## Implementation Steps

### Step 1 — Create test cell

```toml
# cells/apps/vfs-test/Cargo.toml
[package]
name = "app-vfs-test"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "vfs-test"
path = "src/main.rs"

[dependencies]
types = { path = "../../../libs/types" }
api   = { path = "../../../libs/api" }
ostd  = { path = "../../../libs/ostd" }
```

### Step 2 — Test harness macro

```rust
// cells/apps/vfs-test/src/main.rs
macro_rules! assert_vfs {
    ($op:expr, $expected:pat, $msg:literal) => {
        match $op {
            $expected => { println!("[PASS] {}", $msg); PASSED.fetch_add(1, Ordering::Relaxed); }
            other      => { println!("[FAIL] {}: got {:?}", $msg, other); FAILED.fetch_add(1, Ordering::Relaxed); }
        }
    };
}
```

### Step 3 — Add to workspace and disk image

Add `cells/apps/vfs-test` to workspace `Cargo.toml`.
Add `/bin/vfs-test` to `gen_disk.ps1` and the CI bench disk script.

### Step 4 — Add shell alias

Shell: `vfs-test` → `spawn /bin/vfs-test` (runs all tests, shows PASS/FAIL counts).

---

## Todo List

- [ ] Create `cells/apps/vfs-test/Cargo.toml` + `src/main.rs`
- [ ] Implement scenarios 1-7 as automated assertions
- [ ] Add `app-vfs-test` to workspace `Cargo.toml`
- [ ] Add `vfs-test` binary to `gen_disk.ps1` disk image
- [ ] Add `vfs-test` shell builtin (spawns binary)
- [ ] `cargo check -p app-vfs-test` — clean
- [ ] All 7 scenarios pass on QEMU boot

---

## Success Criteria

- [ ] `vfs-test` shell command runs and prints at least 10 PASS lines
- [ ] Zero FAIL lines on a clean boot
- [ ] Quota enforcement test confirms Err(2) on over-limit write
- [ ] Access control test confirms Err(3) on write to `/bin/`
- [ ] Async read test confirms two-opcode protocol works end-to-end
- [ ] All 65 existing integration tests still pass (no regression)
