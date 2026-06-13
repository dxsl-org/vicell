# Track E — Shell M3.1: stderr redirect, tee/sed built-ins, fg/bg, dead utils cleanup

**Status**: Complete  
**Plan dir**: `.agents/260613-2132-e-shell-utilities/`

## Overview

Four focused fixes that close the remaining Shell M3.1 gaps:

| Phase | Description | Status |
|-------|-------------|--------|
| [Phase 1](phase-01-stderr-redirect.md) | Wire `Redirect::StderrTo` execution in `executor.rs` | ✅ Complete |
| [Phase 2](phase-02-tee-sed.md) | Add `tee` built-in + real `sed`; remove dead `cells/apps/utils/` workspace member | ✅ Complete |
| [Phase 3](phase-03-fg-bg.md) | Add `fg`/`bg` built-ins (limitation message, not "command not found") | ✅ Complete |
| [Phase 4](phase-04-shell-test-ci.md) | `shell_test` feature-gate harness + CI integration test | ✅ Complete |

## Key Decisions

- **`cells/apps/utils/` deleted from workspace**: removed from `Cargo.toml` members;
  directory left on disk but excluded from all builds. The 14 stub binaries were
  non-functional (no build.rs, no .ld files, stubs only). Utilities stay as shell built-ins.
- **`2>file` is single-channel**: ViCell shell has one output channel (serial).
  `StderrTo` falls back to stdout redirect when no explicit stdout redirect is present.
  When both `>` and `2>` are present, stdout takes precedence.
- **Cooperative background**: `Ast::Background` runs synchronously — the shell is a
  single-task executor. `fg`/`bg` report this limitation rather than silently hanging.
- **shell_test design A** (test mode inside shell crate via feature flag): avoids
  adding a new Cell, linker script, VA base, and kernel spawn table entry.
  `capture_line` re-uses the existing `SinkGuard`/`OutputSink::Buffer` mechanism.

## Files Modified

| File | Change |
|------|--------|
| `cells/apps/shell/src/executor.rs` | Phase 1: StderrTo fallback; Phase 2: BUILTINS + dispatch for tee/sed; Phase 3: fg/bg dispatch + Background doc; Phase 4: capture_line |
| `cells/apps/shell/src/cmd_fs.rs` | Phase 2: cmd_tee, cmd_sed, sed_replace_first, sed_replace_all |
| `cells/apps/shell/src/main.rs` | Phase 4: shell_test feature branch in main() |
| `cells/apps/shell/src/shell_test.rs` | Phase 4: NEW — 9-scenario test harness |
| `cells/apps/shell/Cargo.toml` | Phase 4: shell_test feature declaration |
| `Cargo.toml` | Phase 2: remove cells/apps/utils workspace member |
| `tests/integration/src/lib.rs` | Phase 4: QemuRunner::boot_rv64 (diskless boot) |
| `tests/integration/tests/shell-utils.rs` | Phase 4: NEW — shell-utils integration test |
| `tests/integration/Cargo.toml` | Phase 4: shell-utils [[test]] entry; fix package name vios→vicell |
| `scripts/build-shell-test-ci.sh` | Phase 4: NEW — CI build script |
| `.github/workflows/ci.yml` | Phase 4: shell-utils job |

## Dependencies

- Prior phases (VFS RamFS /tmp writable, shell pipeline infrastructure) — all done.
- No Law 1 changes (no `libs/api/` or `libs/types/` modifications).
