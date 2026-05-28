# Phase 01 — Workspace Cleanup & Baseline

**Effort:** 20h | **Priority:** P2 | **Status:** complete | **Blockers:** none

## Overview

Eliminate ~19 `profiles` warnings emitted by sub-crate `Cargo.toml` files (profiles only honored at workspace root since cargo 1.x). Verify the newly-merged POSIX shim (PR#6) and async executor (PR#4) compile and integrate cleanly. Establish a 0-warning baseline so subsequent CI (Phase 02) starts from a clean state with `-D warnings`.

## Context Links

- `docs/code-standards.md` — workspace structure rules
- Recent commits: `5b1094b4 feat(libs/api): implement POSIX C Library shim`, `ec1744f8 Merge async-kernel-executor`
- `docs/codebase-summary.md` — crate inventory

## Key Insights

- Cargo `[profile.*]` sections are silently ignored in non-root crates; this triggers a workspace warning each time.
- POSIX shim lives at `libs/api/src/posix.rs`; cells must opt-in via the `posix` feature flag, not unconditional.
- Async executor (`libs/ostd/src/executor.rs`) is the single source of truth — shells & services must use `ostd::executor::spawn`, not their own.

## Requirements

**Functional**
- 0 cargo warnings about profiles
- `cargo build --release --workspace` exits clean
- `cargo check --workspace --target riscv64gc-unknown-none-elf` passes
- POSIX shim symbols (`open`, `read`, `write`, `close`, `lseek`, etc.) resolvable from a cell

**Non-functional**
- No semantic behavior change
- No new dependencies

## Architecture

Profile centralization: move all release/dev/test profile blocks from sub-crates into `Cargo.toml` (workspace root) under `[profile.release]`, `[profile.dev]`, `[profile.bench]`. Sub-crates inherit automatically.

## Related Code Files

**Modify (remove `[profile.*]` blocks):**
- `cells/drivers/disk/Cargo.toml`
- `cells/drivers/gpu/Cargo.toml`
- `cells/drivers/input/Cargo.toml`
- `cells/drivers/net/Cargo.toml`
- `cells/drivers/serial/Cargo.toml`
- `cells/drivers/wasm/Cargo.toml`
- `cells/services/vfs/Cargo.toml`
- `cells/services/compositor/Cargo.toml`
- `cells/services/net/Cargo.toml`
- `cells/services/input/Cargo.toml`
- `cells/services/power/Cargo.toml`
- `cells/services/config/Cargo.toml`
- `cells/apps/init/Cargo.toml`
- `cells/apps/utils/Cargo.toml`
- `cells/apps/test-isolation/Cargo.toml`
- `hal/arch/riscv/Cargo.toml`

**Modify (consolidate profile config):**
- `Cargo.toml` (workspace root) — ensure `[profile.release]`/`[profile.dev]`/`[profile.bench]` contain the union of removed settings

**Verify (no edits expected):**
- `libs/api/src/posix.rs` — POSIX shim API
- `libs/ostd/src/executor.rs` — async executor

**Create:**
- `scripts/check-baseline.sh` — single command CI smoke test (`cargo check --workspace && cargo clippy --workspace -- -D warnings`)

## Implementation Steps

1. Run `cargo check --workspace 2>&1 | grep "profiles"` to enumerate current profile warnings; save list to `scratch-baseline.txt`.
2. Read `Cargo.toml` (workspace root) and record current `[profile.*]` sections.
3. For each sub-crate in the modify list above:
   a. Open Cargo.toml
   b. Remove the entire `[profile.release]`, `[profile.dev]`, `[profile.bench]`, `[profile.test]` blocks
   c. If a setting (e.g. `lto`, `codegen-units`, `opt-level`) is **unique** to that sub-crate, append it to workspace root with a comment `# from <crate>`
4. Re-run `cargo check --workspace` — verify warning count is 0.
5. Verify POSIX shim integration:
   a. `cargo check -p api --features posix`
   b. Grep for callers: `grep -r "use api::posix" cells/`
   c. Sanity-build one cell that imports posix: `cargo build -p shell --features posix` (if shell has the feature) — if it doesn't, document in shell that posix is available behind feature flag.
6. Verify async executor:
   a. `cargo check -p ostd`
   b. `grep -rn "ostd::executor::spawn" cells/` — list call sites
   c. Confirm shell's async loop uses `ostd::executor`, not a hand-rolled one
7. Add `scripts/check-baseline.sh`:
   ```bash
   #!/usr/bin/env bash
   set -euo pipefail
   cargo fmt --all --check
   cargo check --workspace --target riscv64gc-unknown-none-elf -Z build-std=core,alloc
   cargo clippy --workspace --target riscv64gc-unknown-none-elf -Z build-std=core,alloc -- -D warnings
   ```
8. Run `./scripts/check-baseline.sh` locally; fix any clippy issues that fall out (likely unused imports, doc misformat).
9. Update `docs/codebase-summary.md` if profile policy changed (one-line note).
10. Open PR `chore/workspace-baseline`.

## Todo List

- [x] Enumerate current profile warnings, save to scratch-baseline.txt — 16 warnings found
- [x] Record workspace-root profile settings (pre-change snapshot)
- [x] Remove [profile.*] from 16 sub-crate Cargo.tomls — all blocks identical (`panic=abort`, `opt-level=z`); workspace root already covered them
- [x] Merge unique profile overrides into workspace root — no unique overrides; workspace root already had the union
- [x] `cargo check --workspace` — 0 warnings
- [x] Verify POSIX shim compiles (`cargo check -p api --features posix`) — added `posix = []` feature to `libs/api/Cargo.toml`
- [x] List POSIX shim consumers in cells/ — one caller in `cells/runtimes/lua/src/main.rs` (commented-out usage)
- [x] Verify async executor used uniformly — `cells/apps/shell/src/main.rs` uses `ostd::executor::block_on`; `async_utils.rs` uses `yield_now`
- [x] Create `scripts/check-baseline.sh`
- [x] Run baseline script clean — 0 warnings, 0 errors after fixing unused imports across workspace
- [x] Fix any clippy fallout — removed unused imports, dead code, unreachable expressions across all crates
- [x] Update codebase-summary.md if needed — not required; policy change noted in PR
- [x] Open PR `chore/workspace-baseline` — committed as part of multi-phase commit `4f09094f`

## Success Criteria

- `cargo build --release --workspace` finishes with exit 0 and **zero warnings**
- `cargo clippy --workspace -- -D warnings` exits 0
- POSIX shim symbols importable from at least one cell behind a feature flag
- `scripts/check-baseline.sh` succeeds in <60s on a warm cache
- No semantic change to runtime behavior (boot still completes, shell prompt appears)

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Sub-crate had a profile override that mattered (e.g. shell needs `opt-level = "s"`) | Low | Med | Diff release binary size before/after; if regression > 5%, restore at workspace root |
| Clippy uncovers a real bug behind warnings-as-error | Low | Low | Fix in-scope; if non-trivial, defer with `#[allow(...)]` + TODO comment + tracking issue |
| POSIX shim not yet feature-gated cleanly | Med | Low | Add `posix` feature to `libs/api/Cargo.toml`; doc in shim file |

## Security Considerations

- N/A — pure refactor. No public API surface change.
- POSIX shim is unsafe-bearing; ensure cells importing it are still `#![forbid(unsafe_code)]` (the shim itself crosses the boundary, callers don't).

## Rollback

Single PR; `git revert <merge-sha>` restores. Profile settings are inert under cargo, so revert is safe.

## Next Steps

Unblocks Phase 02 (CI/CD) — clean baseline lets `-D warnings` enforce on every push.
