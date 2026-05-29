---
title: "ViOS Complete Implementation Roadmap (v0.2.1 → v1.0.0)"
description: "23-phase plan covering kernel fixes, multi-arch HAL, services, runtimes, security, CI/CD, and community infra for ViOS v1.0 release."
status: pending
priority: P1
effort: 2180h
branch: main
tags: [kernel, hal, infra, feature, security, vios, ipc, vfs, network, compositor, runtime]
created: 2026-05-28
---

# ViOS Complete Implementation Roadmap

**Target:** v0.2.0 (current) → v1.0.0 stable release
**Window:** 2026-05-28 → 2027-06-30 (~13 months)
**Primary target:** RISC-V 64 (QEMU virt), nightly Rust `no_std`
**Secondary targets:** AArch64, x86_64, RV32, AArch32

## Current Baseline (v0.2.0 "Mycelium Alpha")

- 21 crates, ~12,600 LOC Rust
- Working: RV64 HAL (SV39, PLIC, SBI), kernel ELF loader, basic shell REPL, RamFS VFS, config KV store, POSIX shim (PR#6), async executor (PR#4)
- Broken: VirtIO block hangs, keyboard deadlock after 1 keystroke, ARM/x86/RV32 stubs only, no CI, no external ELF loading

## Phase Index

| # | Phase | Effort | Prio | Status | Blockers |
|---|---|---|---|---|---|
| 01 | Workspace Cleanup & Baseline | 20h | P2 | **complete** | none |
| 02 | CI/CD Pipeline | 60h | P1 | **complete** | 01 |
| 03 | Boot Stability & Ring 3 Execution | 40h | P0 | **complete** | none |
| 04 | VirtIO Block Device Fix | 40h | P0 | **complete** | none |
| 05 | Keyboard Input Fix | 20h | P0 | **complete** | none |
| 06 | External ELF Loading from /bin/ | 60h | P1 | **complete** | 03, 04 |
| 07 | VFS FileHandle Passing Between Cells | 30h | P1 | **complete** | 06 |
| 08 | Multi-Arch HAL — ARM AArch64 | 80h | P1 | **complete** | none |
| 09 | Multi-Arch HAL — x86_64 | 80h | P1 | **complete** | none |
| 10 | Lua C Binding via cc Crate | 40h | P1 | **complete** | none |
| 11 | Unit & Integration Tests | 80h | P2 | pending | 03, 04 |
| 12 | Security Audit Infrastructure | 80h | P1 | pending | 02 |
| 13 | Complete VFS Service | 100h | P2 | pending | 04, 06 |
| 14 | Complete Input Service | 80h | P2 | pending | 05, 13 |
| 15 | Complete Network Service | 200h | P2 | pending | 04 |
| 16 | Complete Compositor & GPU | 150h | P2 | pending | 14 |
| 17 | Enhanced Shell & Standard Utilities | 320h | P2 | pending | 13, 14, 15 |
| 18 | Lua & MicroPython Runtime Enhancement | 180h | P2 | pending | 10, 13, 17 |
| 19 | Documentation Automation | 40h | P2 | pending | 02, 11 |
| 20 | Hot Migration & Advanced IPC | 180h | P3 | pending | 06, 13 |
| 21 | RV32 & ARM AArch32 HAL | 160h | P3 | pending | 08 |
| 22 | Benchmarking Suite | 80h | P3 | pending | 1–3 done |
| 23 | Community Infrastructure | 40h | P2 | pending | 19 |
| | **Total** | **~2,180h** | | | |

## Effort Bucketing

| Area | Phases | Hours |
|---|---|---|
| Maintenance + CI | 1, 2 | 80 |
| Core kernel fixes | 3–7 | 190 |
| Multi-arch + tests | 8–11 | 280 |
| Security | 12 | 80 |
| System services | 13–16 | 530 |
| Apps + runtimes | 17–18 | 500 |
| Docs + migration | 19–20 | 220 |
| Advanced + community | 21–23 | 280 |

## Dependency Graph

```
01 ─► 02 ─► 12 ─► 19 ─► 23

03 ┐
04 ┼─► 06 ─► 07 ─► 13 ─► 17 ─► 18
05 ┘                 │
                     ▼
                    14 ─► 16

08, 09, 10  — parallel to 03–07
11           — after 03, 04, parallel with 06–10
15           — after 04
20           — after 06, 13
21           — after 08
22           — after 1–3 complete
23           — after 19
```

## Critical-Path Phases

P0 (BLOCKING for any user-facing demo): **03, 04, 05**
P1 (Required for v1.0): **02, 06, 07, 08, 09, 10, 12**
P2 (v1.0 feature completeness): **11, 13, 14, 15, 16, 17, 18, 19, 23**
P3 (Stretch / v1.x): **20, 21, 22**

## Risk Watchlist (top items, see phase files for full mitigations)

| Risk | Likelihood | Impact | Mitigation Phase |
|---|---|---|---|
| Ring 3 transition unstable on QEMU SBI variants | Med | High | 03 — pin SBI version, golden trace |
| VirtIO descriptor ring alignment subtle | High | High | 04 — adopt `virtio-drivers = 0.7` |
| Async executor wake-storm in shell | Med | Med | 05 — explicit task-state log |
| Hot-migration state divergence | High | Med | 20 — schema versioning + Kani harness |
| Spectre/SAS side-channel limitation | Cert | Med | 12 — document in threat model |
| Lua C source `no_std` patches drift | High | Low | 10 — vendor a pinned Lua release |

## Success Criteria (v1.0)

- All 3 architectures (RV64, AArch64, x86_64) boot to shell prompt in QEMU
- VirtIO block + input + GPU + net all work without hangs
- `cargo test --workspace` passes ≥80% coverage
- CI green on every PR (lint, build matrix, QEMU boot, security)
- Performance targets met: ctx-switch < 100µs, msg < 50µs, syscall < 10µs, kernel+3 services < 10MB
- Public docs site on GitHub Pages, llms.txt index, CONTRIBUTING.md
- ≥10 `good-first-issue` open for community contribution

## Rollback Strategy

Each phase ships in its own feature branch off `main`, merges via PR with CI green.
- Per-phase revert: `git revert <merge-sha>` — safe because phases own disjoint file sets (see "Related Code Files" in each phase)
- Cross-phase invariant breaks: covered by integration tests in Phase 11
- For kernel fixes (03/04/05): keep RamDisk fallback path until Phase 06 makes external loading the default

## Open Questions

- Will we ship a precompiled toolchain image for contributors, or rely on rustup? (decide before Phase 23)
- Smoltcp version pin for Phase 15 — track upstream releases
- ~~Should compositor (Phase 16) use Wayland-style protocol or custom Surface trait?~~ **CLOSED: Custom `ViSurface` trait (Validation Session 1)**

---

## Validation Log

### Session 1 — 2026-05-28
**Trigger:** `/hc:plan validate` after plan creation
**Questions asked:** 6

#### Questions & Answers

1. **[Risk/Assumption]** Phase 03 boot hang — còn tồn tại hay đã fix?
   - Options: Vẫn còn hang | Hang đã fix | Chưa chắc
   - **Answer:** Vẫn còn hang — giữ nguyên scope 40h (fix hang + implement Ring 3)
   - **Rationale:** Phase 03 scope không đổi; cả hai tasks đều cần làm.

2. **[Fact Check / Architecture]** Phase 02 CI — file ci.yml đã tồn tại với 3 bugs (branch `master`, `libkernel` crate, `checkout@v6`)
   - Options: Overwrite toàn bộ | Patch minimal
   - **Answer:** Overwrite + upgrade toàn bộ — fix branch trigger về `main`, upgrade tools, add QEMU boot test + multi-arch matrix
   - **Rationale:** Existing CI never triggers on `main` and tests a non-existent crate; safer to replace cleanly.

3. **[Architecture]** Phase 07 FileHandle — single-owner-moveable vs shared Arc?
   - Options: Single-owner moveable | Shared Arc | Deferred
   - **Answer:** Single-owner moveable — matches ViOS LBI model, simpler reasoning
   - **Rationale:** Sharing modeled by VFS returning multiple independent handles if needed.

4. **[Architecture]** Phase 16 Compositor protocol — Wayland vs custom Surface trait?
   - Options: Custom `ViSurface` trait | Wayland-style | Deferred MVP
   - **Answer:** Custom Surface trait — simpler, native, PDR-aligned
   - **Rationale:** Wayland compat requires external protocol, deferred to v1.x.

5. **[Fact Check FAILED]** `kernel/src/task/task.rs` referenced in Phase 03 + 06 — file doesn't exist
   - Options: task.rs at kernel/src/ level | task/ subdirectory
   - **Answer:** `kernel/src/task.rs` — facade file at kernel/src/ level
   - **Rationale:** Phases 03 and 06 updated with correct path.

6. **[Assumption]** Phase 10 Lua — tarball already present (`lua-5.4.7.tar.gz`), scope?
   - Options: Reduce to ~10h (just fix build.rs) | Keep 40h (full no_std patch + test)
   - **Answer:** Keep 40h — no_std patching of Lua C sources is non-trivial
   - **Rationale:** Tarball present saves download time only; patching + testing still requires full effort.

#### Confirmed Decisions
- Boot hang: still present → Phase 03 scope unchanged (40h)
- CI: full overwrite of ci.yml, fix `master`→`main`, remove `libkernel`, upgrade tools
- FileHandle: single-owner moveable (closed open question)
- Compositor: custom `ViSurface` trait (closed open question)
- Task spawn path: `kernel/src/task.rs` (not `kernel/src/task/task.rs`)
- Lua scope: 40h preserved

#### Action Items
- [x] Fix `kernel/src/task/task.rs` → `kernel/src/task.rs` in phase-03, phase-06
- [x] Add CI overwrite note to phase-02
- [x] Close FileHandle open question in phase-07
- [x] Close compositor protocol open question in phase-16
- [x] Add this Validation Log to plan.md

#### Impact on Phases
- Phase 02: "Create ci.yml" → "Overwrite ci.yml" with full upgrade
- Phase 03: Scope confirmed 40h (hang still present)
- Phase 06: File path corrected
- Phase 07: FileHandle model locked (single-owner moveable)
- Phase 10: Scope confirmed 40h
- Phase 16: Protocol locked (custom ViSurface trait)

---

### Verification Results
- **Tier:** Full (24 phases)
- **Claims checked:** 32
- **Verified:** 30 | **Failed:** 2 | **Unverified:** 0

#### Failures (resolved)
1. [Fact Checker] `kernel/src/task/task.rs` — path not found; actual: `kernel/src/task.rs` → fixed in phase-03, phase-06
2. [Fact Checker] Phase 02 "create `.github/workflows/ci.yml`" — file already exists with bugs → phase-02 updated to "overwrite"
