# ViOS Project Changelog

**Format**: [YYYY-MM-DD] Brief summary of changes, versioned by phase.

---

## [2026-05-28] Phase 01 — Workspace Cleanup (0.2.0 → 0.2.1-dev)

**Changes**:
- Removed all sub-crate `[profile.*]` blocks from `cells/drivers/*/Cargo.toml`, `cells/services/*/Cargo.toml`, and `cells/apps/*/Cargo.toml`
- Consolidated profile configuration at workspace root (`Cargo.toml`)
- Added `posix = []` feature flag to `libs/api/Cargo.toml` for optional POSIX C Library shim
- Workspace now builds with 0 cargo warnings across all targets
- Established zero-warning baseline for subsequent CI enforcement (`-D warnings`)

**Files Modified**:
- `Cargo.toml` (workspace root) — centralized profiles
- `libs/api/Cargo.toml` — added posix feature
- 11 sub-crate `Cargo.toml` files — removed profile blocks

**Impact**: Clean build foundation for Phase 02 CI/CD integration.

---

## [2026-05-28] Phase 02 — CI/CD Pipeline (0.2.1-dev)

**Changes**:
- Created `rust-toolchain.toml` pinning `nightly-2026-05-01` with targets: `riscv64gc-unknown-none-elf`, `aarch64-unknown-none`, `x86_64-unknown-none`
- Implemented `.github/workflows/ci.yml`: 4-job pipeline (lint, build-matrix, qemu-boot, security)
- Implemented `.github/workflows/security.yml`: weekly cargo-audit, cargo-deny, cargo-geiger
- Created `deny.toml` for license scanning and security ban lists
- Added shell scripts: `scripts/qemu-boot-test.sh`, `scripts/qemu-virtio-trace.sh`
- Created GitHub issue templates (bug, feature, refactor) and PR checklist template

**Files Created**:
- `rust-toolchain.toml`
- `.github/workflows/ci.yml`
- `.github/workflows/security.yml`
- `deny.toml`
- `scripts/qemu-boot-test.sh`
- `scripts/qemu-virtio-trace.sh`
- `.github/ISSUE_TEMPLATE/bug_report.md`
- `.github/ISSUE_TEMPLATE/feature_request.md`
- `.github/PULL_REQUEST_TEMPLATE.md`

**Impact**: Automated CI gates all PRs; security scanning weekly; prevents regression across multi-arch targets.

---

## [2026-05-28] Phase 04 — VirtIO Block Device (PARTIAL)

**Changes**:
- **Root Cause Identified**: Limine bootloader does not report MMIO ranges to kernel, causing VirtIO device registers to be unmapped after `activate_paging()`
- **Solution Implemented**:
  - Added explicit identity-mapping of QEMU MMIO regions in `kernel/src/memory/paging.rs`:
    - CLINT: `0x0200_0000`–`0x0200_FFFF`
    - PLIC: `0x0C00_0000`–`0x1000_0000`
    - UART + VirtIO: `0x1000_0000`–`0x1001_0000`
  - Removed duplicate MMIO entries from `kernel/src/boot.rs` FALLBACK_MEMORY_MAP (now contains only RAM regions; MMIO handled by paging.rs)
  - Memset safety verified in `kernel/src/intrinsics.rs`

**Files Modified**:
- `kernel/src/memory/paging.rs` — added explicit MMIO identity-mapping block to `init_kernel_paging()`
- `kernel/src/boot.rs` — removed duplicate MMIO entries from FALLBACK_MEMORY_MAP

**Status**: Root cause fixed. Full I/O testing deferred to Phase 06 (External ELF Loading) integration.

**Impact**: Unblocks VirtIO device discovery and interrupt delivery; kernel no longer panics on MMIO access.

---

## [2026-06-03] Phase F — FAT16 Hardening (Complete)

**Changes**:
- **Phase 1 (OP_WRITE Header Widening)**:
  - `cells/apps/shell/src/cmd_fs.rs:263-279` — `write_file()` refactored with 4-byte header: `[opcode][path_len:u8][content_len:u16 LE][path][content]`
  - `cells/services/vfs/src/main.rs:340-358` — OP_WRITE arm updated to parse `u16::from_le_bytes([buf[2], buf[3]])` for content length, offset 4 for path
  - Effective write cap increased from 253 bytes (before) to 512 - 4 - path_len (now), enabling large-content writes in single message
- **Phase 2 (OP_UNLINK for /data/ FAT16)**:
  - `cells/services/vfs/src/main.rs:287-290` — `unlink_fat16()` helper added; routes `/data/` prefixed paths to FAT16 deletion
  - OP_UNLINK arm (line 383) refactored with `/data/` branch
  - Shell already sends OP_UNLINK via 2-byte header; no client change
- **Phase 3 (Subdirectories under /data/)**:
  - `cells/services/vfs/src/main.rs:242` — Added `DataDir<'a>` type alias for cleaner helper signatures
  - `cells/services/vfs/src/main.rs:258-330` — Added `split_last()`, `ensure_dir_chain()`, `fat16_mkdir()` helpers
  - Refactored `write_fat16()` to use `ensure_dir_chain()` for mkdir -p parent creation, then `create_file()` with full relative path
  - Refactored `read_fat16()` to use `open_file(rel_path)` for full path traversal (fatfs handles '/'-separated paths natively)
  - Refactored `unlink_fat16()` to use `remove(rel_path)` for nested path deletion
  - OP_MKDIR arm (line 371) refactored with `/data/` branch routing to `fat16_mkdir`, else to RamFS `vfs.mkdir`
  - Nested write/read/delete now fully functional: `/data/sub/f` creates `sub/` dir, writes `f`, reads back, deletes
- **Phase 4 (Block Syscall Capability Gate)**:
  - `kernel/src/task/syscall.rs:62` — Added `VFS_TASK_ID: usize = 3` constant with TODO and ServiceLookup cross-ref
  - `Syscall::BlkRead`, `BlkWrite`, `BlkFlush` arms (lines 1095, 1112, 1072) — Each gated with `if caller_id != VFS_TASK_ID { log::warn + return Err(PermissionDenied) }`
  - `Syscall::Shutdown` (line 1080) — Explicitly untouched, remains open to all
  - Security improvement: raw block I/O syscalls (500/501/503) now restricted to VFS cell (task 3); prevents arbitrary sector reads/writes

**Files Modified**:
- `cells/apps/shell/src/cmd_fs.rs` — 4-byte OP_WRITE header
- `cells/services/vfs/src/main.rs` — FAT16 hardening: unlink, mkdir, nested path traversal
- `kernel/src/task/syscall.rs` — Block I/O capability gate

**Status**: Complete. All 17 integration tests pass; 4 phases independent + fully integrated.

**Integration Tests Added**:
- `vfs_fat16_large_write` — validates 4-byte header widening (>253-byte content per message)
- `vfs_fat16_unlink` — flat-file deletion via OP_UNLINK
- `vfs_fat16_subdir` — nested directory creation, write, read, delete
- `vfs_fat16_deep_nesting` — 3+ level mkdir -p chains

**Impact**:
- VFS FAT16 now feature-complete for session-local (same-boot) writes with directory support
- 4-byte header removes chunking bottleneck for large writes (up to 512-byte messages)
- Unlink + mkdir on /data/ enable destructive operations (scripts can clean, recreate state)
- Block I/O gating closes privilege escalation hole; non-VFS cells can no longer corrupt disk

---

## [2026-06-03] Phase G — FAT16 Completion (0.2.1-dev)

**Changes**:
- **Phase 1 (can_block_io TCB flag)**: Replaced boot-order-fragile `VFS_TASK_ID == 3` hardcode with per-cell `can_block_io: bool` flag set at spawn time for `/bin/vfs`
  - `kernel/src/task/tcb.rs:126` — added field, default false
  - `kernel/src/loader.rs:73-83` — grant logic; sets true when spawned path ends `/bin/vfs`
  - `kernel/src/task/syscall.rs:70-82` — added `caller_has_block_io()` helper
  - `kernel/src/task/syscall.rs:1082,1109,1130` — updated all 3 block-I/O gates (BlkFlush, BlkRead, BlkWrite)
  - Removed `VFS_TASK_ID` constant entirely
- **Phase 2 (OP_RMDIR for FAT16)**: Extended OP_RMDIR to route `/data/` paths to FAT16, enabling empty dir deletion
  - `cells/services/vfs/src/main.rs:425-436` — OP_RMDIR arm now branches on path prefix, reuses `unlink_fat16()` (DRY)
- **Phase 3 (Negative block-I/O test)**: Added security regression test asserting non-VFS cells cannot call raw block I/O
  - `cells/apps/shell/src/cmd_sys.rs:72-81` — `cmd_blkio_test()` shell command
  - `cells/apps/shell/src/executor.rs` — registered `"blktest"` dispatch arm
  - `tests/integration/tests/boot.rs:486-510` — `block_io_denied_non_vfs` integration test
- **Phase 4 (Subdir reboot persistence test)**: Validated FAT16 subdirectory writes survive power cycle
  - `tests/integration/tests/boot.rs:512-568` — `vfs_fat16_subdir_persistence` integration test

**Files Modified**:
- `kernel/src/task/tcb.rs` — `can_block_io` field
- `kernel/src/loader.rs` — grant logic in `spawn_from_path`
- `kernel/src/task/syscall.rs` — `caller_has_block_io()` helper + gate updates
- `cells/services/vfs/src/main.rs` — OP_RMDIR branch for `/data/`
- `cells/apps/shell/src/cmd_sys.rs` — `cmd_blkio_test()` command
- `cells/apps/shell/src/executor.rs` — dispatch registration
- `tests/integration/tests/boot.rs` — 2 new integration tests

**Status**: Complete. 4 independent phases, all integrated. 19/19 integration tests pass.

**Integration Tests Added**:
- `block_io_denied_non_vfs` — verifies capability gate rejects non-VFS block I/O syscalls
- `vfs_fat16_subdir_persistence` — validates nested-dir writes survive reboot (mirrors Phase E pattern)

**Impact**:
- Block I/O capability now boot-order-independent; safer, more modular design
- FAT16 rmdir enables cleanup scripts; `/data/` directory lifecycle complete
- Security regression test locks in privilege separation; accidental grants caught immediately
- Subdir persistence proved end-to-end; FAT16 is now a durable storage backend
- Foundation for Phase G (capability tokens, reboot persistence of subdirs, ACPI/PSCI)

---

## [2026-06-03] Phase E — Hardening + Reboot Persistence (Complete)

**Changes**:
- **Hardening (Safety Fixes)**:
  - `cells/services/vfs/src/block_stream.rs:87` — SeekFrom::Current now validates result ≥ 0 before u64 cast to prevent underflow→arbitrary sector seek
  - `kernel/src/task/syscall.rs:1072, 1084` — BlkRead/BlkWrite handlers reject sectors ≥ CELL_TABLE_BASE_LBA (82,000) to prevent cell-corrupted kernel bootstrap table
- **Clean Shutdown Path**:
  - `kernel/src/task/syscall.rs:256` — Added `Shutdown` variant to internal `Syscall` enum
  - `kernel/src/task/syscall.rs:1109–1121` — SBI SRST handler (M-mode shutdown via OpenSBI)
  - `kernel/src/task/syscall.rs:1203` — Numeric map 502 → Shutdown
  - `libs/ostd/src/syscall.rs:80–98` — `sys_shutdown()` -> ! wrapper
  - `cells/apps/shell/src/cmd_sys.rs:69–72` — `cmd_shutdown()` built-in
  - `cells/apps/shell/src/executor.rs:160` — "shutdown" command arm registered
- **Test Harness Improvements**:
  - `tests/integration/src/lib.rs:145–165` — `wait_for_natural_exit(timeout_secs)` method allows graceful QEMU exit (disk flush) before reboot
- **Integration Test**:
  - `tests/integration/tests/boot.rs:362–409` — `vfs_fat16_reboot_persistence` test (write marker → shutdown → reboot → read-back)
- **Critical Bug Fix**:
  - Removed pre-parser echo handler from `cells/apps/shell/src/shell.rs::dispatch()` that was splitting by whitespace and bypassing redirect parser
  - Root cause of echo-redirect failures (`echo X > /path` printed to console instead of writing file)
  - Fix verified by Phase E integration test

**Files Modified**:
- `cells/services/vfs/src/block_stream.rs`
- `kernel/src/task/syscall.rs`
- `libs/ostd/src/syscall.rs`
- `cells/apps/shell/src/cmd_sys.rs`, `executor.rs`, `shell.rs`
- `tests/integration/src/lib.rs`, `tests/integration/tests/boot.rs`

**Status**: Complete. All 14 integration tests pass; FAT16 write durability across reboot proven.

**Impact**: 
- Closes two Phase D code-review findings (safety)
- Proves FileSystem persistence across power cycle (critical for real OS)
- Fixes shell echo-redirect bug (enables `>` redirection in scripts)
- Unblocks Phase F features dependent on clean shutdown (ACPI/PSCI, power loss recovery)

---

## [2026-05-28] Phase 05 — Keyboard Input Fix (Complete)

**Changes**:
- **Root Cause Identified**: VirtIO input IRQ was never acknowledged, leaving `InterruptStatus` set; PLIC re-fired interrupt forever (interrupt storm) → kernel hung
- **Solution Implemented**:
  - Added `pub static INPUT_DEVICE_IRQ` constant and `pub fn ack_irq(irq: u32) -> bool` to `kernel/src/task/drivers/virtio_input.rs`
  - Expanded `vi_handle_virtio_irq()` in `kernel/src/task/drivers/virtio_blk.rs` to dispatch to both block and input devices
  - Established IRQ numbering pattern: QEMU VirtIO MMIO slot `i` → IRQ `i+1` (applies to all VirtIO device types)
  - Input device properly re-arms virtqueue and publishes buffers back to available ring after consuming events

**Files Modified**:
- `kernel/src/task/drivers/virtio_input.rs` — added IRQ constant and acknowledgment function
- `kernel/src/task/drivers/virtio_blk.rs` — expanded interrupt dispatch to include input devices

**Status**: Complete. Verified and ready for Phase 2 shell interaction testing.

**Impact**: Shell now reliably reads multiple consecutive keystrokes; no deadlock on subsequent input. Foundational fix enabling interactive REPL.

---

## See Also

- **project-roadmap.md** — Live phase tracking and milestone definitions
- **system-architecture.md** — Updated with VirtIO IRQ dispatch pattern and MMIO mapping strategy
- **code-standards.md** — Development rules and project structure
- **codebase-summary.md** — Current file structure and LOC counts

---

## Version History

| Version | Date | Phase(s) | Status |
|---------|------|----------|--------|
| 0.2.0 | 2026-05-01 | Phase 0 (Alpha) | Stable baseline |
| 0.2.1-dev | 2026-06-03 | Phases 01–23, C/D/E/F complete | In progress |
| 0.2.1 | TBD | Phase 1 + Phases C/D/E/F complete | Pending |
| 0.3.0 | 2026-09-30 | Phases 2–3 + Phase G | Planned |
| 1.0.0 | 2027-03-31 | Phases 4+ | Planned |

---

## [2026-06-03] Phase C — VFS RamFS Write + Shell Echo Redirect (Complete)

**Changes**:
- **Phase 1 (VFS Endpoint Fix)**: Fixed shell's hardcoded `VFS_ENDPOINT = 2` (silently misrouted to user_hello); replaced with dynamic `sys_service_lookup("vfs")` wrapper (hardcoded fallback 3)
  - Added `sys_service_lookup` ostd syscall wrapper for ServiceLookup (opcode 100)
  - Updated shell `cmd_fs.rs` to use `vfs_endpoint()` helper for all VFS IPC
  - Verified correct routing: shell → VFS cell (task 3) for all path operations
- **Phase 2 (OP_WRITE Handler)**: Implemented RamFS file write in VFS service
  - Added `write_file(&mut self, path: &str, content: &[u8]) -> bool` to VfsManager
  - Implemented `OP_WRITE (opcode 4)` handler: 3-byte header `[4][path_len][content_len]`, validates `/tmp/` prefix guard, writes to RamFS tree
  - Added `OP_READ (opcode 8)` handler: reads file bytes back from RamFS (used by vcat built-in)
  - Returns 0x00 on success, 0x01 on error (path outside /tmp, parent missing, etc.)
- **Phase 3 (Echo Built-in + Redirect)**: Added real echo built-in and stdout redirect capture for persistent writes
  - Implemented `cmd_echo` built-in in shell (replaces spawn of `/bin/echo`)
  - Wired `StdoutTo` redirect to intercept echo output: builds bytes, sends OP_WRITE to VFS, skips console print
  - Added `write_file()` client function with 3-byte header protocol matching VFS handler
  - Added `vcat` built-in for VFS-backed file read (reads via OP_READ)
  - Integration with shell executor: early-return for echo+redirect, log-only for other built-ins with redirects (deferred)
- **Phase 4 (Integration Test)**: End-to-end round-trip test validates all phases together
  - Added `vfs_write_echo_redirect` integration test: boot → echo PHASE_C_WRITE > /tmp/test.txt → vcat /tmp/test.txt → assert read-back
  - All 12 integration tests pass ✅

**Files Modified**:
- `libs/ostd/src/syscall.rs` — added `sys_service_lookup` wrapper
- `cells/apps/shell/src/cmd_fs.rs` — fixed VFS_ENDPOINT, added vfs_endpoint(), write_file() client, read_file_vfs() client
- `cells/apps/shell/src/commands.rs` — added cmd_echo_to_vec(), cmd_echo(), cmd_vcat() built-ins
- `cells/apps/shell/src/executor.rs` — registered echo in dispatch_builtin, added StdoutTo redirect capture for echo
- `cells/services/vfs/src/main.rs` — added write_file(), get_file_data() to VfsManager, implemented OP_WRITE + OP_READ handlers
- `tests/integration/tests/boot.rs` — added vfs_write_echo_redirect test

**Status**: Complete. RamFS write functional for session-local `/tmp/` writes. FAT32 persistence deferred to Phase D.

**Impact**: 
- Shell output now persists in-session: `echo TEXT > /tmp/file` writes to VFS RamFS
- `vcat` built-in reads back VFS-stored files
- `/tmp/` prefix guard prevents unauthorized writes
- Foundation for Phase D (FAT16 disk integration) and Phase E+ (reboot-persistent storage)

---

## [2026-06-03] Phase D — FAT16 Write Persistence on VirtIO Block Device (Complete)

**Changes**:
- **Phase 1 (Block I/O Syscalls)**: Exposed VirtIO block device via raw syscalls 500 (BlkRead) and 501 (BlkWrite) without modifying stable ABI
  - Added private `syscall_raw` helper in `libs/ostd/src/syscall.rs` to bypass `ViSyscall` enum
  - Added `sys_blk_read(sector, &mut [u8;512]) -> bool` and `sys_blk_write(sector, &[u8;512]) -> bool` to ostd
  - Added `Syscall::BlkRead` and `Syscall::BlkWrite` variants to kernel (internal enum only)
  - Added kernel handlers in `handle_syscall` with `validate_user_buf` checks
  - Mapped 500/501 in numeric fallback of `vios_syscall_dispatch`
  - Verified against `viVirtIOBlk.read_sector()`/`write_sector()` trait methods
- **Phase 2 (FAT16 Format)**: Created disk formatter for LBA 0–81919 (before cell table at LBA 82000)
  - Created `tools/mkfat16.py`: in-place FAT16 formatter with 81920 sectors, 8 sec/cluster, 10225 clusters
  - Integrated into `gen_disk.ps1` step 3c (after blank image, before cell-table append)
  - BPB validation: magic 0x55AA at offset 510, type label "FAT16   " at 54–61
  - Cluster count verified in FAT16 window (4085–65524)
- **Phase 3 (BlockStream + fatfs Mount)**: Enabled FAT16 in VFS service via syscalls
  - Created `cells/services/vfs/src/block_stream.rs`: fatfs IoBase adapter over syscall 500/501
  - Implemented BlockStream::read/write with sector-granular RMW for sub-sector ops
  - Implemented BlockStream::seek (Start/Current) with End→Err fallback (not needed in Phase D)
  - Added `fatfs` git dependency to VFS (deduped with kernel)
  - Mount FAT16 at VFS startup; fallback to RamFS-only if mount fails
- **Phase 4 (VFS Routing)**: Branched OP_WRITE and OP_READ on path prefix
  - Added `/data/` prefix detection in OP_WRITE handler (routes to `write_fat16` helper)
  - Implemented `write_fat16`: remove existing file (avoid append/truncate edge case) + create-fresh with content
  - Added `/data/` prefix detection in OP_READ handler (routes to `read_fat16` helper)
  - Implemented `read_fat16`: open file, loop-read up to 480 bytes, send response
  - `/tmp/` paths unchanged (continue to route through RamFS)
- **Phase 5 (Integration Test)**: Validated full stack in single-session write → read round trip
  - Added `vfs_fat16_write_read` integration test: boot → write `PHASE_D_PERSIST` to `/data/test.txt` → read via vcat
  - Asserts FAT16 mount log detection
  - Verifies marker returned in read-back
  - All 13 integration tests pass ✅

**Files Created**:
- `tools/mkfat16.py` — in-place FAT16 formatter
- `cells/services/vfs/src/block_stream.rs` — fatfs I/O adapter

**Files Modified**:
- `kernel/src/task/syscall.rs` — added BlkRead/BlkWrite syscall support
- `libs/ostd/src/syscall.rs` — added sys_blk_read/write
- `cells/services/vfs/Cargo.toml` — added fatfs dependency
- `cells/services/vfs/src/main.rs` — FAT16 mount + routing branches
- `gen_disk.ps1` — added mkfat16.py step
- `tests/integration/tests/boot.rs` — added vfs_fat16_write_read test

**Status**: Complete. FAT16 write-persistence functional for session-local `/data/` writes. Reboot persistence deferred to Phase E.

**Impact**:
- Shell writes to `/data/` now persist on VirtIO block device: `echo TEXT > /data/file` survives session (within same boot)
- VFS transparently routes `/data/*` through FAT16 filesystem
- `/tmp/` writes remain volatile (RamFS); `/data/` writes durable (block device)
- Foundation for Phase E (reboot persistence, subdirs, sector-range capability gates)

**Known Limitations**:
- Writes are volatile (RamFS only; lost on reboot)
- Kernel FS (`/bin`, `/etc`) and VFS RamFS (`/tmp`) are separate stores; `cat` reads kernel FS, `vcat` reads VFS
- Multi-KB writes truncated to 253-byte client buffer (chunking deferred)
- No append (>>) or other redirect modes (2>); only StdoutTo working for echo

**Next Phase**:
- Phase D: FAT32 disk write integration + `/tmp` → FAT32 redirect

---

## [2026-06-03] Phase A–B — Network TCP Data-Path (Complete)

**Changes**:
- **Phase A (prior)**: CONNECT / SEND / RECV / CLOSE opcodes wired; TCP client functional
- **Phase B**: Extended with HTTP/1.0 GET client and socket state introspection
  - Added `SOCKET_STATE (0x19)` opcode to net cell: query live TCP state (1-byte encoding)
  - Implemented `curl` binary: HTTP/1.0 GET client with URL parsing, response accumulation, FIN detection
  - Disk-build integration: added `/bin/nc` and `/bin/curl` to disk cell table
  - Integration test: `network_curl_http_get` with host HTTP server end-to-end validation

**Files Modified**:
- `cells/services/net/src/poll_driver.rs` — added SOCKET_STATE constant (0x19)
- `cells/services/net/src/main.rs` — added tcp_state_byte() helper, SOCKET_STATE handler
- `cells/apps/net-tools/src/bin/curl.rs` — full HTTP/1.0 GET client (replaced stub)
- `gen_disk.ps1` — build app-net-tools, add /bin/nc and /bin/curl to cell table
- `tests/integration/src/lib.rs` — added spawn_http_server()
- `tests/integration/tests/boot.rs` — added network_curl_http_get test

**Status**: Phase A + B complete. Phase C (VFS write for persistent responses) planned.

**Impact**: ViOS can now fetch HTTP responses from external servers; network tooling usable from shell.

---

## [2026-06-03] Status Update — Phases 10, 14, 15, 16, 18, 20 Verified (0.2.1-dev)

**Verification**:
- Phase 10 (External ELF Loading): ✅ `spawn_from_path` verified, shell/config/vfs load from `/bin/`
- Phase 14 (Keyboard): ✅ Multi-key input, no deadlock, history + arrow keys working
- Phase 15 (Network): ✅ DHCP verified (10.0.2.15 assignment), data-path stubs (CONNECT/SEND/RECV return 0xFF)
- Phase 16 (Compositor): ✅ Basic framebuffer, GPU opt-in (setup_framebuffer gates integration)
- Phase 18 (MicroPython): ✅ Runtime REPL verified, 256KB heap, VFS I/O FFI working
- Phase 20 (HotSwap): ✅ 5-step orchestrator verified, shell/config/vfs hot-swap tested, state transfer working

**Documentation Updates**:
- Updated all docs to reflect v0.2.1-dev status
- Corrected HAL status: RV64 full, AArch64 + x86_64 full (Ring-3 smoke), RV32 + AArch32 stubs
- Updated kernel LOC: ~8,700 (from ~5,300)
- Codebase total: ~21,473 LOC
- MicroPython marked as verified (not "planned")
- HotSwap marked as implemented (not "planned")

---

## [2026-05-29] Phases 11–23 — Major Feature Wave (0.2.1-dev)

**Changes** (key deliverables across all phases):

### Libraries / API
- `libs/api/src/input.rs` — `InputEvent`, `KeyEvent`, `KeySym`, `Modifiers`, `MouseButton` types
- `libs/api/src/display.rs` — `Rect`, `PixelFormat`, `SurfaceCap`, compositor IPC opcodes
- `libs/api/src/benchmark.rs` — `BenchReport` with p50/p99 percentiles + JSON output
- `libs/api/src/syscall.rs` — added `RecvTimeout`, `SendGather`, `RecvScatter`, `HotSwap`, `GpuFlush`
- `libs/ostd/src/repl.rs` — shared readline + history state machine
- `libs/ostd/src/syscall.rs` — `sys_get_time`, `sys_gpu_flush`, `sys_hotswap`, `sys_recv_timeout`, scatter/gather wrappers

### Kernel
- `kernel/src/task/tcb.rs` — `Recv::deadline` field for timeout IPC
- `kernel/src/task/syscall.rs` — dispatchers for HotSwap, GpuFlush, RecvTimeout, SendGather, RecvScatter
- `kernel/src/cell/cap_registry.rs` — `expires_at` lease + `grant_depth` enforcement + `alloc_with_lease`
- `kernel/src/cell/hotswap.rs` — 5-step live Cell replacement orchestrator
- `kernel/src/task/drivers/virtio_net.rs` — VirtIO NIC kernel driver (mirrors virtio_blk)

### Services / Cells
- `cells/services/vfs/` — OP_MKDIR/RMDIR/UNLINK IPC, `readdir` trait, `ViStateTransfer` (quota table)
- `cells/services/input/` — full US QWERTY translator, modifier state, focus dispatcher
- `cells/services/net/` — smoltcp TCP/IPv4 + VirtIO NIC IPC + DHCP client
- `cells/services/compositor/` — software blending, damage tracking, 30 FPS render loop, `GpuFlush` integration
- `cells/runtimes/lua/` — multi-line REPL, history, `bindings_io` VFS I/O FFI
- `cells/services/config/` — `ViStateTransfer` for KV map
- `cells/apps/shell/` — parser (pipe/redirect/background/sequence), executor, jobs, history, aliases, `ViStateTransfer`
- `cells/apps/bench/` — 4-scenario benchmark cell (ctx-switch, IPC, syscall, footprint)
- `cells/apps/sys-tools/` — ps, env, uname, date, free, kill, shutdown, hotswap
- `cells/apps/net-tools/` — ping, curl, nc, wget (stubs for Phase 15 data-path)
- `cells/apps/utils/` — wc, head, tail, grep, sort, sed, cp, mv, rm, mkdir, touch

### Infrastructure
- `.github/workflows/perf.yml` — weekly benchmark CI with regression gate
- `scripts/format-disk.ps1` — FAT32 disk image generator
- `scripts/compare-bench-results.sh` — rolling-median regression detector
- `gen_disk.ps1` — updated to bake all Phase 17b utility binaries

### Docs
- `docs/vfs-api.md`, `docs/input-api.md`, `docs/display-api.md`, `docs/network-api.md`
- `docs/hotswap-guide.md`, `docs/scripting-guide.md`, `docs/performance-report.md`
- `docs/ROADMAP.md`, `docs/FAQ.md`, `docs/CONTRIBUTING.md` (polished)
- `scripts/dev-setup.sh`, `scripts/dev-setup.ps1`

**Impact**: All 23 plan phases are at least `partial`; the system compiles clean with zero new errors.

