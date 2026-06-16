# Phase 4: Block Syscall Capability Gate

## Context Links
- Plan: [plan.md](plan.md)
- Dispatch arms: `kernel/src/task/syscall.rs:1095` (BlkRead),
  `:1112` (BlkWrite), `:1072` (BlkFlush)
- Numeric fallback (enum construction only): `syscall.rs:1237-1240`
- `caller_id` source: `syscall.rs:1249` (`super::current_task_id()`), passed
  into `handle_syscall(caller_id, ..)` at `:1257`
- ServiceLookup hardcodes vfs=3: `syscall.rs:657-671`
- Sector ceiling guard already present: `syscall.rs:1098`, `:1115`

## Overview
- **Priority:** P2
- **Status:** pending
- **Description:** Raw syscalls 500/501/503 (BlkRead/Write/Flush) dispatch to the
  VirtIO disk regardless of caller. Restrict to the VFS cell (task 3). 502
  (Shutdown) stays open to all.

## Key Insight (corrects the brief)
- The brief said add the check "in the numeric fallback section" (syscall.rs:1224-1246).
  **That is wrong:** the fallback only *constructs* the `Syscall` enum; `caller_id`
  is not in scope there — it is computed at `:1249`, AFTER the match. The gate
  MUST go inside `handle_syscall`, whose first parameter IS `caller_id`, at the
  `Syscall::BlkRead` / `BlkWrite` / `BlkFlush` arms.
- `502 => Syscall::Shutdown` is a distinct enum variant with no sector args —
  already outside 500/501/503; leaving it ungated requires no action.

## Data Flow
```
cell ecall (a7=500, a0=sector, a1=buf_ptr)
  └ ViCell_syscall_dispatch: fallback builds Syscall::BlkRead{..}
       └ caller_id = current_task_id()        (syscall.rs:1249)
            └ handle_syscall(caller_id, BlkRead{..})
                 └ NEW: if caller_id != VFS_TASK_ID { warn + return Err }   ← gate
                      └ sector ceiling guard → viVirtIOBlk.read_sector
```

## Related Code Files
**Modify:** `kernel/src/task/syscall.rs` only — the three Blk* arms.
**Create:** none.

## Implementation Steps

1. Add a module constant near `MAX_USER_BUF` (syscall.rs:62):
   ```rust
   /// Task ID permitted to issue raw block-device syscalls (500/501/503).
   /// VFS is task 3 in the standard boot order (init=1, user_hello=2, vfs=3) —
   /// see the ServiceLookup table (`"vfs" => 3`) which hardcodes the same value.
   /// TODO Phase G: replace this task-ID check with a capability token so the
   /// gate survives boot-order changes.
   const VFS_TASK_ID: usize = 3;
   ```

2. At the **top of each** `Syscall::BlkRead`, `Syscall::BlkWrite`,
   `Syscall::BlkFlush` arm (before the existing body), insert:
   ```rust
   if caller_id != VFS_TASK_ID {
       log::warn!(
           "blk syscall denied: task {} != VFS_TASK_ID {} (boot order changed?)",
           caller_id, VFS_TASK_ID
       );
       return Err(SyscallError::PermissionDenied);
   }
   ```
   - BlkRead arm body starts at syscall.rs:1095 (insert before the sector guard at :1098).
   - BlkWrite arm body starts at syscall.rs:1112 (insert before :1115).
   - BlkFlush arm body starts at syscall.rs:1072 (insert before the `viVirtIOBlk.flush()` call).

3. **Do NOT touch** `Syscall::Shutdown` (syscall.rs:1080) — remains open.

4. `cargo check -p kernel`; `cargo clippy -p kernel -- -D warnings`.

## Todo List
- [ ] Add `VFS_TASK_ID` constant with TODO + ServiceLookup cross-ref comment
- [ ] Gate BlkRead arm (warn + PermissionDenied)
- [ ] Gate BlkWrite arm
- [ ] Gate BlkFlush arm
- [ ] Confirm Shutdown untouched
- [ ] `cargo check -p kernel` + clippy
- [ ] Re-run all FAT16 boot.rs tests (VFS still operates from task 3)

## Success Criteria
- `cargo check` + clippy clean.
- **No regression:** `vfs_fat16_write_read`, `vfs_fat16_reboot_persistence`,
  Phase 2/3 tests all still pass — proves the VFS cell (task 3) is still allowed.
- Denial path: `PermissionDenied` maps to `usize::MAX` at the ABI level
  (syscall.rs:1267), so a non-VFS caller gets `-1`.

## Risk Assessment
| Risk | L | I | Mitigation |
|------|---|---|-----------|
| Hardcoded task 3 breaks if a kernel task spawns before init | Med | High | `log::warn!` on every rejection makes the break loud on serial; comment cross-refs ServiceLookup (`"vfs" => 3`, syscall.rs:663). Both hardcodes must change together in Phase G. |
| VFS task ID ≠ 3 in some build → all /data ops silently fail | Low | High | The no-regression test suite (write/read/persistence) fails immediately if VFS is gated out — turns a silent break into a red test. |
| Gating Shutdown by mistake | Low | Med | Explicit step 3 + Shutdown is a separate variant; reviewer checks it is untouched. |

## Negative Test (deferred — documented, not implemented)
A true negative test needs a shell built-in that issues `BlkRead` directly and
asserts `-1`. No such command exists and adding one is out of scope for Phase F.
**Verification for this phase = compile + clippy + the no-regression suite.**
The positive case (VFS still works) transitively proves the `caller_id == 3`
branch; the negative branch is covered by code review of the three identical
guard insertions. Phase G should add the negative test alongside the cap token.

## Security Considerations
- Closes the hole where any cell could read/write/flush arbitrary disk sectors
  (bounded only by the existing `CELL_TABLE_BASE_LBA` ceiling).
- Defense-in-depth: the sector ceiling guard (syscall.rs:1098/1115) stays — the
  cap gate is an additional, earlier check.
- This is a task-ID check, NOT a capability token — explicitly interim. The
  TODO + warn make the temporary nature visible.

## Next Steps
- Fully independent (kernel-only). Can land first or last.
- Phase G: replace `VFS_TASK_ID` with a `CapPerms::BLOCK_IO` capability resolved
  via `caller_id → cell_id → cap table` (pattern already exists for File caps,
  syscall.rs:822-834).

---

## Evidence (2026-06-03, Complete)

**Code Changes Verified:**
- `kernel/src/task/syscall.rs:62` — Added `VFS_TASK_ID: usize = 3` constant with TODO and ServiceLookup cross-ref comment
- `Syscall::BlkRead` arm (line 1095) — Added gate: `if caller_id != VFS_TASK_ID { warn + return Err(PermissionDenied) }`
- `Syscall::BlkWrite` arm (line 1112) — Added identical gate
- `Syscall::BlkFlush` arm (line 1072) — Added identical gate
- `Syscall::Shutdown` arm (line 1080) — **Explicitly untouched**, remains open

**Compilation:**
- `cargo check -p kernel`: ✅ clean
- `cargo clippy -p kernel -- -D warnings`: ✅ clean

**Test Results:**
- `cargo test --test integration boot::vfs_fat16_write_read` — ✅ pass (VFS task 3 still permitted BlkWrite)
- `cargo test --test integration boot::vfs_fat16_reboot_persistence` — ✅ pass (VFS task 3 still permitted BlkRead/Write/Flush across shutdown/reboot)
- `cargo test --test integration boot::vfs_fat16_large_write` — ✅ pass (compound BlkRead/Write across nested ops)
- `cargo test --test integration boot::vfs_fat16_unlink` — ✅ pass
- `cargo test --test integration boot::vfs_fat16_subdir` — ✅ pass (mkdir + BlockStream I/O)

**Integration Suite:**
- All 17/17 integration tests pass
- No regression in any phase; VFS (task 3) retains full access
- Security gate: non-VFS callers will receive `PermissionDenied` (mapped to `-1` at ABI)
- **Negative test deferred** (no shell command to issue BlkRead directly); covered by code review + positive no-regression suite
- `log::warn!` guard makes any future boot-order breakage immediately visible on serial
