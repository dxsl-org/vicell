# Phase 1: `can_block_io` TCB Flag (replaces VFS_TASK_ID hardcode)

## Context Links
- `kernel/src/task/tcb.rs:91` — `Task` struct; `Task::new` at line 130
- `kernel/src/task/syscall.rs:73` — `VFS_TASK_ID` constant (to remove)
- `kernel/src/task/syscall.rs:1081,1108,1129` — the three `caller_id != VFS_TASK_ID` gates
- `kernel/src/loader.rs:44` — `spawn_from_path` (REAL grant point; see correction below)
- `kernel/src/task.rs:29` — `pub(crate) static SCHEDULER`
- `kernel/src/task/scheduler.rs:28` — `pub tasks: BTreeMap<usize, Box<Task>>`

## Overview
- **Priority:** P2
- **Status:** pending
- **Description:** Replace the boot-order-fragile `VFS_TASK_ID = 3` constant with a per-TCB
  `can_block_io: bool` flag set at spawn time when the spawned path is `/bin/vfs`.

## Key Insights (verified 2026-06-03)
- **CRITICAL CORRECTION:** The input brief said the grant goes in `task.rs:spawn_from_path`.
  That function does **not exist**. `task.rs:spawn_from_file` (line 268) is a stub returning
  `Err(ViError::NotSupported)`. The actual path-spawn is `crate::loader::spawn_from_path`
  (loader.rs:44), reached via the `SpawnFromPath` syscall handler (syscall.rs:786 → loader.rs:801).
  The grant MUST be applied in `loader.rs:spawn_from_path`, on the `tid` returned by
  `spawn_from_mem` at loader.rs:68.
- `loader.rs:spawn_from_path` already has the `path: &str` in scope and already calls
  `spawn_from_mem(&elf_bytes, name, CellId(0), …)` which returns `Result<usize, ViError>`.
- `SCHEDULER.lock().as_mut()` + `sched.tasks.get_mut(&tid)` is the established pattern
  (e.g. task.rs:213, syscall.rs:594) — `tasks` is `BTreeMap<usize, Box<Task>>`, so
  `get_mut(&tid)` returns `Option<&mut Box<Task>>`; field access auto-derefs.
- Three gates to replace, all identical shape (syscall.rs:1081 BlkFlush, 1108 BlkRead, 1129 BlkWrite).

## Data Flow
```
init cell ── SpawnFromPath("/bin/vfs") ──▶ syscall.rs:786
                                              │
                                              ▼
                              loader::spawn_from_path(path)        loader.rs:44
                                              │  read ELF, reloc
                                              ▼
                              spawn_from_mem(...) ─▶ tid           loader.rs:68
                                              │
                          [NEW] if path ends_with "/bin/vfs":
                                SCHEDULER → tasks[tid].can_block_io = true
                                              │
                                              ▼
later:  vfs cell ── BlkRead/Write/Flush ──▶ handle_syscall(caller=tid)
                                              │
                              caller_has_block_io(tid) → true → allow
        any other cell ─────────────────────┘ → false → PermissionDenied
```

## Related Code Files
**Modify:**
- `kernel/src/task/tcb.rs` — add field + default
- `kernel/src/loader.rs` — set flag after `spawn_from_mem`
- `kernel/src/task/syscall.rs` — add helper, replace 3 gates, remove constant

**Create:** none. **Delete:** none.

## Implementation Steps

### 1a. Add field to `Task` (tcb.rs)
After `pending_future: Option<SyscallFuture>,` (line 126), inside the struct:
```rust
    /// Grants access to raw block-device syscalls (500/501/503).
    /// Set at spawn time for `/bin/vfs`; false for every other cell.
    /// Phase H replaces this with a formal `CapPerms::BLOCK_IO` capability token.
    pub can_block_io: bool,
```
In `Task::new` (line 131-152), add to the struct literal (after `pending_future: None,`):
```rust
            can_block_io: false,
```
Note: `Task` has `#[allow(dead_code)]` — field reads happen only via the scheduler, so no
extra `#[allow]` needed.

### 1b. Grant the flag at the real spawn site (loader.rs)
In `spawn_from_path`, change the final `spawn_from_mem` call (lines 67-69) from a tail return
into a `let tid = …?;` then grant + return:
```rust
    // Spawn via the existing in-memory spawn path (ELF parse + segment map).
    let tid = crate::task::spawn_from_mem(&elf_bytes, name, CellId(0), alloc::vec::Vec::new())
        .map_err(|_| ViError::OutOfMemory)?;

    // Grant raw block-I/O to the VFS service only. Boot-order-independent
    // replacement for the former `VFS_TASK_ID == 3` hardcode (Phase G).
    if path.ends_with("/bin/vfs") {
        if let Some(sched) = crate::task::SCHEDULER.lock().as_mut() {
            if let Some(task) = sched.tasks.get_mut(&tid) {
                task.can_block_io = true;
            }
        }
    }
    Ok(tid)
```
`SCHEDULER` is `pub(crate)` (task.rs:29) — reachable from `loader.rs` as `crate::task::SCHEDULER`.

### 1c. Replace the gates (syscall.rs)
Add a free helper near the top of the file (after the constants, e.g. below line 73 where
`VFS_TASK_ID` was — but place it before `handle_syscall`):
```rust
/// Returns true if the calling task holds block-I/O permission.
/// Replaces the former `caller_id == VFS_TASK_ID` boot-order check (Phase G).
/// TODO Phase H: fold into a formal `CapPerms::BLOCK_IO` capability token.
fn caller_has_block_io(caller_id: usize) -> bool {
    super::SCHEDULER
        .lock()
        .as_ref()
        .and_then(|sched| sched.tasks.get(&caller_id))
        .map(|t| t.can_block_io)
        .unwrap_or(false)
}
```
Then replace each of the three gates. Example for BlkFlush (line 1081-1084):
```rust
            if !caller_has_block_io(caller_id) {
                log::warn!("BlkFlush denied: task {} lacks block-I/O capability", caller_id);
                return Err(SyscallError::PermissionDenied);
            }
```
Apply the same change to BlkRead (1108) and BlkWrite (1129), keeping each op's name in the
warning. **Lock-ordering note:** `caller_has_block_io` takes `SCHEDULER` then drops it before
the handler proceeds to acquire BLOCK_DEVICE — no nested lock across the FS read (mirrors the
deadlock-avoidance comment in early.rs:108).

### 1d. Remove the dead constant
Delete `VFS_TASK_ID` and its doc block (syscall.rs:67-73). The cross-referenced ServiceLookup
table (`"vfs" => 3`, syscall.rs:671) is a SEPARATE concern (IPC routing, not block-I/O) — leave
it untouched.

### 1e. Compile
`cargo check -p vios-kernel`

## Todo
- [ ] 1a: add `can_block_io` field + `Task::new` default
- [ ] 1b: grant flag in `loader.rs:spawn_from_path`
- [ ] 1c: add `caller_has_block_io`, replace 3 gates
- [ ] 1d: remove `VFS_TASK_ID` constant + doc block
- [ ] 1e: `cargo check -p vios-kernel` passes

## Success Criteria
- Kernel compiles with no `VFS_TASK_ID` references (`rg VFS_TASK_ID kernel/` → 0 hits).
- All three block-I/O gates call `caller_has_block_io`.
- VFS (spawned via `/bin/vfs`) gets `can_block_io = true`; verified end-to-end by the
  existing `vfs_fat16_write_read` test still passing (it requires VFS to reach the disk).

## Risk Assessment
| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| R1-1: VFS spawned by a path NOT ending `/bin/vfs` → flag never set → all `/data` writes break | Low | High | Confirm `gen_disk.ps1`/init spawns exactly `/bin/vfs` (early.rs:100 doc says so). Regression caught immediately by `vfs_fat16_write_read`. |
| R1-2: `get_mut(&tid)` races a concurrent terminate | Very Low | Low | Grant runs synchronously right after spawn inside the same call; task cannot have exited yet. |
| R1-3: lock-order regression (SCHEDULER held across BLOCK_DEVICE) | Low | High | Helper drops the guard before returning; no FS call inside the lock scope. |

## Security Considerations
- Tightens, not loosens: default `false` means a forgotten grant fails CLOSED (denies I/O),
  the safe failure mode. The old hardcode failed OPEN if boot order shifted (task 3 might not
  be VFS). This is a net security improvement.
- No new ABI surface (`libs/api` untouched) → no 2x-confirm gate triggered (Law 1).

## Evidence

**Verified 2026-06-03**:
- `kernel/src/task/tcb.rs:126` — `can_block_io: bool` field added, default `false`
- `kernel/src/loader.rs:73-83` — grant logic added; sets flag when `path.ends_with("/bin/vfs")`
- `kernel/src/task/syscall.rs:70-82` — `caller_has_block_io()` helper added (replaces constant)
- `kernel/src/task/syscall.rs:1082,1109,1130` — all 3 block-I/O gates (BlkFlush, BlkRead, BlkWrite) updated
- `VFS_TASK_ID` constant removed entirely (0 grep hits in `kernel/`)
- `cargo build -p vios-kernel -r` passes with 0 errors (stripe warning is expected)

## Next Steps
Phase 3's negative test (`block_io_denied_non_vfs`) validates this gate from userspace.
Phase 1 must land before Phase 3.
