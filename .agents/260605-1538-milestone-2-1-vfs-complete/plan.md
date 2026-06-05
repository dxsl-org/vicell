# Milestone 2.1 — Complete VFS Service

**Status**: 📋 PLANNED  
**Priority**: P0  
**Target**: 2026-08-30  
**Effort**: ~3.5 weeks  
**Created**: 2026-06-05

---

## Goal

Complete the VFS service by wiring existing quota infrastructure, adding subdirectory listing, implementing capability-based access control, adding non-blocking async read, and creating a formal integration test suite.

---

## Scope Decision: What's "Complete VFS"

**Research findings (2026-06-05):**

| Original Goal | Decision | Rationale |
|--------------|----------|-----------|
| FAT32 support | **SKIP** — FAT16 is correct | FAT32 needs ≥65525 clusters (~256MB+); disk_v3.img is 40MB. `fatfs` crate auto-detects; no action needed until disk grows past 256MB |
| Directory creation/deletion/listing | **PARTIAL** — add subdirectory listing | mkdir/rmdir/unlink already work; missing: recursive listing beyond root level |
| File permissions (read/write/execute) | **Capability-based, not POSIX** | SAS OS: CellId + path prefix rules. No uid/gid/mode bits. No persistent storage on FAT |
| Async file operations (non-blocking) | **Two-opcode protocol** | `ReadAsync` → `PendingHandle` → `Poll`. No executor changes. Simplest viable path |
| Disk quota tracking | **Wire existing tracker** | `QuotaTracker` exists and works; just not called in write path |

---

## Phases

| # | File | Status | Effort | Description |
|---|------|--------|--------|-------------|
| 1 | [phase-01-quota-enforcement.md](phase-01-quota-enforcement.md) | 📋 PLANNED | 2 days | Wire QuotaTracker to write/append paths |
| 2 | [phase-02-directory-listing.md](phase-02-directory-listing.md) | 📋 PLANNED | 3 days | Recursive directory listing, stat on dirs |
| 3 | [phase-03-capability-access.md](phase-03-capability-access.md) | 📋 PLANNED | 4 days | CellId-based path access control |
| 4 | [phase-04-async-read.md](phase-04-async-read.md) | 📋 PLANNED | 5 days | Non-blocking ReadAsync + Poll opcodes |
| 5 | [phase-05-integration-tests.md](phase-05-integration-tests.md) | 📋 PLANNED | 3 days | Formal integration test suite |

**Execution order**: 1 → 2 → 3 → 4 → 5. Phases 1-2 are independent; Phase 3 depends on understanding from Phase 1-2; Phase 4 is independent of Phase 3.

---

## Current State (2026-06-05)

| Component | Status |
|-----------|--------|
| FAT16 mount at `/data/` | ✅ Working |
| RamFS at `/tmp/` | ✅ Working |
| All 9 VfsRequest opcodes | ✅ Implemented |
| QuotaTracker struct | ✅ Exists in `quota.rs` |
| Quota enforcement in write path | ❌ Not wired |
| Subdirectory listing | ❌ `ListDir` only works at root level |
| File permissions | ❌ None — only path-prefix auth |
| Async/non-blocking reads | ❌ All synchronous |
| Integration tests | ❌ Manual shell tests only |
| FAT32 support | ✅ Not needed at current scale |

---

## Success Criteria

- [ ] Writing past quota returns `VfsResponse::Err` with quota-exceeded code
- [ ] `ListDir("/data/subdir")` correctly enumerates FAT16 subdirectory entries
- [ ] A Cell without write capability to `/data/` gets `PermissionDenied`
- [ ] `ReadAsync("/data/large.bin")` returns immediately with a handle; `Poll(handle)` returns data when ready
- [ ] Integration test suite: at least 10 test scenarios covering the above
- [ ] All 65 existing integration tests still pass
