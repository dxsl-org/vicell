# Contributing to Cellos

Welcome! Cellos is a `no_std` Rust OS built on a **Cellular Single Address Space**
architecture — a different design point from traditional Linux/Windows-style OSes.
This guide gets you from zero to your first merged PR.

---

## Table of Contents

1. [DCO Sign-off](#dco-sign-off)
2. [Setup](#setup)
3. [Code Standards](#code-standards)
4. [Your First PR — Step by Step](#your-first-pr--step-by-step)
5. [Commit Messages](#commit-messages)
6. [Submitting a PR](#submitting-a-pr)
7. [Review Checklist](#review-checklist)
8. [Where to Start](#where-to-start)
9. [Getting Help](#getting-help)

---

## DCO Sign-off

All commits must include a `Signed-off-by:` line certifying you agree to the
[Developer Certificate of Origin](DCO). This is a lightweight process — no CLA,
no copyright assignment. **You keep full ownership of your contribution.**

```bash
git commit -s -m "feat(kernel): your message"
# Automatically appends: Signed-off-by: Your Name <your@email.com>
```

A GitHub Actions check enforces this on every PR. See [DCO](DCO) for the full text.

---

## Setup

**Automated (recommended):**

```bash
# Linux / macOS
./scripts/dev-setup.sh

# Windows (PowerShell 7+)
.\scripts\dev-setup.ps1
```

Both scripts are idempotent — safe to run again after a toolchain update.

**Manual prerequisites:**

| Tool | Version | Install |
|------|---------|---------|
| Rust nightly | pinned in `rust-toolchain.toml` | `rustup toolchain install nightly` |
| `rust-src` component | — | `rustup component add rust-src` |
| QEMU RV64 | ≥ 8.0 | `apt install qemu-system-misc` / `brew install qemu` |
| mtools | any | needed for `scripts/format-disk.ps1` |

For detailed setup help — including a **Common Errors table** — see
[docs/ONBOARDING.md](docs/ONBOARDING.md).

---

## Code Standards

### The 8 Laws (non-negotiable)

1. **Interface is Sacred** — changes to `libs/api/` or `libs/types/` require 2×
   confirmation. These define the ABI between kernel and Cells.
2. **Owned Buffers for Async** — `async fn process(data: Box<[u8]>) -> Box<[u8]>`,
   never `async fn process(data: &mut [u8])`.
3. **Multi-Architecture Awareness** — use `VAddr`, `PAddr` from `libs/types`;
   never hard-code 32/64-bit pointer sizes.
4. **No `unsafe` in Cells** — `#![forbid(unsafe_code)]` in every Cell crate.
   Kernel and HAL `unsafe` must have `// SAFETY:` comments.
5. **No `mod.rs`** — use `foo.rs` parallel to `foo/` directory.
6. **`Vi` prefix for public types** — `ViFileSystem`, `ViDriver`, `ViError`.
7. **`dyn Trait` at boundaries** — `Arc<dyn ViDriver + Send + Sync>` for shared
   resources; `Box` for single-owner.
8. **Implement `Drop`** — all resources must clean up explicitly via `Drop`.

### Smoke check before every PR

```bash
./scripts/check-baseline.sh
# runs: cargo fmt --check, cargo check --workspace, cargo clippy -D warnings
```

See [docs/code-standards.md](docs/code-standards.md) for full style guidance.

---

## Your First PR — Step by Step

A concrete walk-through using the shell as an example. Adapt file paths for your
actual change.

### 1 — Find something to work on

Browse issues labelled
[`good-first-issue`](../../issues?q=label%3Agood-first-issue).  Each issue lists
context, acceptance criteria, and relevant files.

### 2 — Read the relevant spec

Check [CLAUDE.md](CLAUDE.md) → "Before Coding — Read Specifications" to find which
`docs/0N-*.md` file covers your area.  Spend 5–10 minutes on it.

### 3 — Create a branch

```bash
git checkout main
git pull --ff-only
git checkout -b feat/shell-my-command   # or fix/, docs/, test/, refactor/
```

### 4 — Implement

Follow the patterns already in the file you are editing.  For a shell command:

```bash
# find the command dispatch table
grep -n '"ls"' cells/apps/shell/src/commands.rs
```

Add your implementation next to a similar existing command.

### 5 — Verify

```bash
# Must pass before submitting
cargo check --workspace
cargo fmt --all --check
cargo clippy --workspace -- -D warnings

# Run host-side unit tests (types + api crates)
cargo test -p types -p api --target x86_64-pc-windows-msvc   # Windows
cargo test -p types -p api --target x86_64-unknown-linux-gnu  # Linux
```

### 6 — Commit

```bash
git add <files>
git commit -s -m "feat(shell): add my-command

Brief explanation of what this does and why.

Closes #NNN"
```

### 7 — Push and open PR

```bash
git push -u origin feat/shell-my-command
```

Then open a Pull Request on GitHub.  The PR template will prompt you for:

- What changed and why
- How you tested it
- Screenshot / log output (for user-visible changes)

### 8 — Address review feedback

Make changes, `git push` again (no force-push needed unless requested), and reply
to every comment — even with "Done" — so reviewers know each item was addressed.

---

## Commit Messages

Use [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <subject>       ← max 72 chars, imperative mood

<body>                           ← optional; explain WHY not WHAT

<footer>                         ← optional; "Closes #N", "BREAKING CHANGE: ..."
```

**Types:** `feat` · `fix` · `docs` · `refactor` · `test` · `chore` · `perf`

**Scopes:** `kernel` · `hal` · `vfs` · `shell` · `input` · `net` · `types` · `api`

---

## Submitting a PR

- **One logical change per PR.** Separate refactors from features.
- **CI must be green.** Do not open a PR you know is broken (use a draft PR
  instead).
- **Tests.** Add at least one test or document why one is not needed.
- **Docs.** Update relevant `docs/` files if behaviour changes.
- **No secrets.** Never commit `.env`, credentials, or private keys.

---

## Review Checklist

Reviewers check these before approving:

- [ ] Follows the 8 Laws above
- [ ] `// SAFETY:` present on every `unsafe` block
- [ ] Public items have rustdoc (`///`)
- [ ] `cargo check --workspace` clean
- [ ] `cargo clippy` clean (no new warnings)
- [ ] Tests added or reason given
- [ ] No `mod.rs` files introduced
- [ ] Law 2 (no `&mut [u8]` across async boundaries) obeyed

---

## UI Subsystem (ViUI)

ViUI v2 is Cellos's native UI toolkit — `no_std`, Reactive Signal Tree, MIT-licensed.

**DSL syntax note**: The `.vi` file format intentionally mirrors [Slint](https://slint.dev)'s
`.slint` syntax for developer familiarity (migration cost ≈ zero). The ViUI *runtime engine*
is a cleanroom Rust implementation — Signal<T> reactive cells, CPU/GPU command buffer
renderer — with no code derived from Slint's GPL-3 codebase. Syntax is not copyrightable
(EU ECJ: SAS v. WPL 2012; US: Lotus v. Borland 1996).

**GPU renderer note**: `GpuRenderer<E>` uses a command-list pattern (record phase + execute
phase), not direct GPU hardware calls. `CpuExecutor` replays commands via the CPU framebuffer
rasterizer. A hardware `HwGpuExecutor` for embedded 2D GPUs (Mali DE, PowerVR) is deferred
to G2. On QEMU VirtIO GPU, CPU rasterization is the only supported path.

Key files: `libs/viui/` · Spec: `docs/specs/14-viui.md` · Phases: `.agents/260608-*/`

---

## Where to Start

- [`good-first-issue`](../../issues?q=label%3Agood-first-issue) — curated issues
  with full context
- [docs/ONBOARDING.md](docs/ONBOARDING.md) — full setup guide + common errors
- [docs/FAQ.md](docs/FAQ.md) — architecture questions answered
- [docs/ROADMAP.md](docs/ROADMAP.md) — where the project is headed

---

## Getting Help

- **GitHub Discussions** — Q&A, ideas, show-and-tell
- **GitHub Issues** — bug reports and feature requests
- Review the [Common Errors table](docs/ONBOARDING.md#common-errors--fixes) before
  opening a new thread

We aim to respond to PRs and issues within 3 business days.
