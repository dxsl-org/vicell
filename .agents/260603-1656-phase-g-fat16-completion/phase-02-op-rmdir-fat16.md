# Phase 2: OP_RMDIR for FAT16

## Context Links
- `cells/services/vfs/src/main.rs:38` — `OP_RMDIR` opcode = 6
- `cells/services/vfs/src/main.rs:425-430` — OP_RMDIR arm (routes only to `vfs.rmdir`)
- `cells/services/vfs/src/main.rs:315-322` — `unlink_fat16` (already does `root_dir().remove(rel)`)
- `cells/services/vfs/src/main.rs:241` — `type DataFs`
- `cells/services/vfs/src/main.rs:170` — `VfsManager::rmdir` (RamFS, empty-dir only)

## Overview
- **Priority:** P2
- **Status:** pending
- **Description:** Route OP_RMDIR for `/data/` paths to the FAT16 volume so `/data/` dirs
  can be deleted. Currently only RamFS (`/tmp`) dirs are removable.

## Key Insights (verified 2026-06-03)
- OP_RMDIR arm (425-430) calls **only** `vfs.rmdir(p)` — confirmed: `/data/` dirs cannot be
  deleted today. Matches the OP_MKDIR/OP_UNLINK arms which DO branch on `/data/`.
- `unlink_fat16` (315) already implements exactly `fs.root_dir().remove(rel)`. In fatfs,
  `Dir::remove` removes a file OR an empty directory, and errors `DirectoryIsNotEmpty` on a
  non-empty dir — i.e. it ALREADY has POSIX-rmdir semantics for the dir case.
- **DRY decision:** Rather than add a near-duplicate `rmdir_fat16`, reuse `unlink_fat16` for
  the `/data/` branch. The function name `unlink_fat16` is slightly misleading for dirs, but
  the brief's proposed `rmdir_fat16` body is byte-identical to `unlink_fat16`. Two options
  below — pick **Option A** (KISS/DRY).

## Data Flow
```
shell `rmdir /data/x` ─▶ cmd_rmdir ─▶ OP_RMDIR IPC ─▶ vfs main loop (main.rs:425)
                                                          │
                              [NEW] if p.starts_with("/data/"):
                                    unlink_fat16(fat_fs.as_ref(), p)   ── fatfs remove()
                              else:
                                    vfs.rmdir(p)                       ── RamFS
                                                          │
                                          reply 0x00 ok / 0x01 err ─▶ shell
```

## Related Code Files
**Modify:** `cells/services/vfs/src/main.rs` (OP_RMDIR arm only). **Create/Delete:** none.

## Implementation Steps

### Option A — reuse `unlink_fat16` (RECOMMENDED, DRY)
Replace the OP_RMDIR arm (main.rs:425-430):
```rust
                    OP_RMDIR => {
                        if let Some(p) = path {
                            // fatfs `remove()` deletes an empty dir and errors on a
                            // non-empty one — same POSIX-rmdir semantics as vfs.rmdir().
                            let ok = if p.starts_with("/data/") {
                                unlink_fat16(fat_fs.as_ref(), p)
                            } else {
                                vfs.rmdir(p)
                            };
                            ostd::syscall::sys_send(sender, if ok { b"\x00" } else { b"\x01" });
                        }
                    }
```
This mirrors the OP_UNLINK arm (431-438) exactly and adds zero new functions.

### Option B — dedicated helper (only if reviewer wants a distinct name)
Add after `fat16_mkdir` (main.rs:332) — semantically identical to `unlink_fat16`, exists
solely for call-site readability:
```rust
/// Remove an EMPTY `/data/[sub/]DIR` from the FAT16 volume.
/// Returns false on non-empty dir, mirroring POSIX rmdir (fatfs errors natively).
fn rmdir_fat16(fs: Option<&DataFs>, path: &str) -> bool {
    unlink_fat16(fs, path) // delegates — fatfs remove() handles both cases
}
```
then call `rmdir_fat16(...)` in the arm. **Prefer Option A** unless a code reviewer objects.

### Compile
`cargo check -p service-vfs`

## Todo
- [ ] Update OP_RMDIR arm to branch on `/data/` → `unlink_fat16` (Option A)
- [ ] `cargo check -p service-vfs` passes

## Success Criteria
- `rmdir /data/x` removes an empty `/data/x` directory and returns `0x00`.
- `rmdir` on a non-empty `/data/` dir returns `0x01` (no silent recursive delete).
- `/tmp` rmdir behavior unchanged (still routes to `vfs.rmdir`).
- Validated by the integration step below.

## Integration Validation (manual or via Phase 3/4 harness)
`mkdir /data/rmdir_test` → `rmdir /data/rmdir_test` → `mkdir /data/rmdir_test` again.
If the second `mkdir` succeeds, the dir was genuinely deleted. (Not a standalone `#[test]`
in this phase — covered by ad-hoc QEMU run; a formal test is out of scope per plan boundary.)

## Risk Assessment
| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| R2-1: fatfs `remove` recursively deletes a non-empty dir | Very Low | High | Phase F established `remove` traverses natively and errors on non-empty. Behavior identical to existing `unlink_fat16` which is already trusted. |
| R2-2: path like `/data/` (empty rel) deletes the volume root | Low | High | `unlink_fat16` guards `Some(n) if !n.is_empty()` (main.rs:318) → returns false for `/data/`. |
| R2-3: name confusion (`unlink_fat16` used for dirs) | Low | Low | Inline comment in the arm explains the shared semantics; Option B available if reviewer prefers. |

## Security Considerations
- No new syscall/ABI surface. `/data/` path-prefix guard prevents traversal outside the volume.
- Non-empty-dir protection preserved (no accidental recursive wipe).

## Evidence

**Verified 2026-06-03**:
- `cells/services/vfs/src/main.rs:425-436` — OP_RMDIR arm now routes `/data/` to `unlink_fat16()` (Option A, DRY)
- Mirrors OP_UNLINK (main.rs:437-446) structure exactly
- `cargo build -p service-vfs -r` passes with 0 errors

## Next Steps
Independent of all other phases. Can land before or after Phase 1.
