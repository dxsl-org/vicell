---
name: project-phase-c-vfs-write-complete
description: Phase C (VFS RamFS write + shell echo redirect) completed 2026-06-03; 12/12 integration tests pass
metadata:
  type: project
---

## Phase C: VFS RamFS Write + Shell Echo Redirect — COMPLETE

**Completed:** 2026-06-03  
**Status:** All 4 sub-phases delivered, 12/12 integration tests passing

### What Was Delivered

**Phase 1 — VFS Endpoint Fix:**
- Fixed shell's hardcoded `VFS_ENDPOINT = 2` → dynamic `sys_service_lookup("vfs")` wrapper
- Added ostd syscall wrapper for ServiceLookup (opcode 100)
- Fallback: hardcoded 3 (boot order: init=1, user_hello=2, vfs=3)

**Phase 2 — OP_WRITE Handler:**
- Added `write_file()` method to VfsManager in VFS service cell
- Implemented OP_WRITE (opcode 4): 3-byte header `[opcode][path_len][content_len]`
- `/tmp/` prefix guard: rejects writes outside `/tmp/` directory
- Returns 0x00 ok, 0x01 error
- Added OP_READ (opcode 8) handler for read-back support

**Phase 3 — Echo Built-in + Redirect:**
- Added `cmd_echo` real built-in to shell (replaces external `/bin/echo`)
- Wired `StdoutTo` redirect: captures echo output to `Vec<u8>`, sends OP_WRITE to VFS, skips console print
- Added `write_file()` IPC client with matching 3-byte header protocol
- Added `vcat` built-in for VFS-backed file reads (via OP_READ)
- Design: echo+redirect captured early in exec_cmd, other built-ins with redirects remain log-only (out of scope)

**Phase 4 — Integration Test:**
- End-to-end test: boot → `echo PHASE_C_WRITE > /tmp/test.txt` → `vcat /tmp/test.txt` → assert
- All 12 integration tests pass ✅

### Key Design Decisions

1. **Separate kernel FS and VFS RamFS:** The test uses `vcat` (VFS OP_READ) to read back, not `cat` (kernel FS `sys_open`), because the stores are separate. This is intentional — kernel FS loads from embedded disk image, VFS RamFS is in-process. Phase D will integrate them.

2. **3-byte header protocol:** `[opcode][path_len][content_len]` included in design spec to avoid zero-padding ambiguity. Client and server both implement it.

3. **ServiceLookup wrapper:** Added for dynamic endpoint resolution, not just hardcoded 3, to support future cells and Phase 3+ shell redirect.

4. **Volatile RamFS write:** Phase C does not persist to disk. Writes are lost on reboot. Phase D (FAT32 integration) will add persistence.

### Files Modified

- `libs/ostd/src/syscall.rs` — added sys_service_lookup
- `cells/apps/shell/src/cmd_fs.rs` — vfs_endpoint(), write_file(), read_file_vfs()
- `cells/apps/shell/src/commands.rs` — cmd_echo, cmd_echo_to_vec, cmd_vcat
- `cells/apps/shell/src/executor.rs` — StdoutTo redirect capture for echo
- `cells/services/vfs/src/main.rs` — write_file(), OP_WRITE + OP_READ handlers
- `tests/integration/tests/boot.rs` — vfs_write_echo_redirect test

### Test Evidence

- `cargo check` passes for all modified crates (ostd, service-vfs, app-shell)
- `cargo test ... -- --test-threads=1` → 12/12 pass ✅
- QEMU manual verification:
  - `echo hello world` prints to console ✅
  - `echo PHASE_C_WRITE > /tmp/test.txt` writes silently ✅
  - `vcat /tmp/test.txt` reads back `PHASE_C_WRITE` ✅
  - `echo X > /etc/passwd` rejected by /tmp guard ✅

### Documentation Updates

- Updated `docs/system-architecture.md` Current Status section: added Phase C, marked RamFS write complete, FAT32 deferred to Phase D
- Added comprehensive Phase C entry to `docs/project-changelog.md` with scope, impact, and known limitations
- Updated plan.md and all phase-*.md files with Evidence sections and completion dates

### Known Limitations (Phase C Scope)

- Writes are volatile (RamFS only, no disk)
- Kernel FS and VFS RamFS are separate stores
- Client buffer limited to 253 bytes (multi-KB writes require chunking, Phase D+)
- Only `echo` can redirect to VFS; other built-ins remain log-only
- No append (>>), stdin (<), or stderr (2>) redirect modes

### Next Phase

**Phase D: FAT32 Disk Write + Persistent `/tmp/`**
- Integrate FAT32 write to disk (already have FAT32 read)
- Add `/tmp` → FAT32 redirect so files persist across reboots
- Expand redirect capture to all built-ins (not just echo)

**Why Phase C Succeeded**

1. **Comprehensive plan with detailed phases** — split 1-phase work into 4 concrete sub-phases, each with clear dependencies, implementation steps, and success criteria
2. **Design-first approach** — resolved protocol ambiguities (3-byte header) before implementation
3. **Correct root causes** — correctly identified that echo was not a built-in, that VFS_ENDPOINT was wrong, and that kernel FS ≠ VFS RamFS
4. **Integration test-first** — wrote test before some implementation, which forced clarity on what "works" meant
5. **Evidence-based completion** — each phase has Evidence section showing what was actually built, not just "marked done"

---

*Sync completed by project-manager agent; all artifacts in place; ready for Phase D planning.*
