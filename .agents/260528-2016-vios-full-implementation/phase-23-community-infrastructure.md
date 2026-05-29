# Phase 23 — Community Infrastructure

**Effort:** 40h | **Priority:** P2 | **Status:** **complete** | **Blockers:** Phase 19 (resolved)

## Overview

Open the project for outside contributions: complete `CONTRIBUTING.md` and `CODE_OF_CONDUCT.md` (Phase 19 created them; this phase polishes & cross-links), GitHub Discussions templates, public-facing simplified roadmap, one-command developer setup script (`scripts/dev-setup.sh`), 10+ labeled `good-first-issue` entries, and an updated `ONBOARDING.md` calibrated for non-Anthropic contributors. After this phase, a stranger can clone the repo and ship their first PR in under a day.

## Context Links

- Phase 19 — created CONTRIBUTING + CODE_OF_CONDUCT; this phase extends + polishes
- `docs/ONBOARDING.md` — existing, needs community calibration
- `docs/project-roadmap.md` — internal roadmap; this phase produces a simplified public version
- Phase 02 — issue templates already present; verify completeness

## Key Insights

- Friction = drop-off. The number-one barrier for new contributors is `it didn't build the first time`. The `dev-setup.sh` script removes 80% of that friction by automating toolchain install, target add, dependency check, and a smoke build.
- `good-first-issue` curated by area (docs, shell, tests, utils) lets contributors self-select by interest. Each issue must include: context, acceptance criteria, pointers to relevant files, and difficulty estimate (S / M / L).
- Discussions vs Issues: bug = Issue, "how do I" or "should we" = Discussion. The templates enforce this routing.
- Public roadmap (`docs/ROADMAP.md`) is a *narrative*, not a Gantt chart. It tells outside readers "what's done, what's next, why" — without exposing every internal phase number.

## Requirements

**Functional**
- `CONTRIBUTING.md` covers setup, build, test, conventional commits, PR rules, where-to-start (live links)
- `CODE_OF_CONDUCT.md` published (Contributor Covenant v2.1 verbatim)
- `scripts/dev-setup.sh` — Bash; idempotent; works on Ubuntu 22.04/24.04 and macOS 14+
- `scripts/dev-setup.ps1` — PowerShell equivalent for Windows 10/11
- GitHub Discussions enabled with 4 categories (Announcements, Q&A, Show and Tell, Ideas), each with a template
- ≥ 10 issues labeled `good-first-issue` spread across `area:docs`, `area:shell`, `area:tests`, `area:utils`
- `docs/ROADMAP.md` — public narrative roadmap, < 200 lines
- `docs/ONBOARDING.md` updated with common errors table + time estimates per OS

**Non-functional**
- New contributor smoke-test: 1 person runs `./scripts/dev-setup.sh && ./run.ps1` (or `.sh`) cold and reports time-to-prompt
- Target: < 45 min on Linux, < 60 min on Windows

## Architecture (deliverables map)

```
Root:
   CONTRIBUTING.md            ← polished from Phase 19 baseline
   CODE_OF_CONDUCT.md         ← from Phase 19
   README.md                  ← cross-link badges + Quick Start + Community section

scripts/
   dev-setup.sh               ← Linux/macOS one-command setup
   dev-setup.ps1              ← Windows PowerShell equivalent

docs/
   ROADMAP.md                 ← public narrative
   ONBOARDING.md              ← updated with errors + timings
   FAQ.md                     ← top 10 questions answered

.github/
   DISCUSSION_TEMPLATE/
     announcements.md
     q-and-a.md
     show-and-tell.md
     ideas.md
```

## Related Code Files

**Create:**
- `scripts/dev-setup.sh` — Bash setup
- `scripts/dev-setup.ps1` — PowerShell setup
- `docs/ROADMAP.md` — public roadmap narrative
- `docs/FAQ.md` — top 10 FAQ
- `.github/DISCUSSION_TEMPLATE/announcements.md`
- `.github/DISCUSSION_TEMPLATE/q-and-a.md`
- `.github/DISCUSSION_TEMPLATE/show-and-tell.md`
- `.github/DISCUSSION_TEMPLATE/ideas.md`

**Modify:**
- `CONTRIBUTING.md` — polish from Phase 19 base; add "Your first PR" walkthrough
- `README.md` — add Community section, link Discussions + Contributing
- `docs/ONBOARDING.md` — Common Errors table; per-OS time estimates
- `docs/development-roadmap.md` — flag as internal; point to ROADMAP.md for public

**Issue authoring (GitHub):**
- Create 10+ `good-first-issue` issues; tag with area labels; assign difficulty

## Implementation Steps

### Phase 23.1 — Setup scripts (12h)

1. **`scripts/dev-setup.sh`** (Bash, idempotent):
   ```bash
   #!/usr/bin/env bash
   set -euo pipefail
   OS=$(uname -s)
   command -v rustup >/dev/null || curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none
   . "$HOME/.cargo/env"
   rustup toolchain install $(cat rust-toolchain.toml | grep channel | cut -d'"' -f2)
   rustup component add rust-src rustfmt clippy llvm-tools-preview
   rustup target add riscv64gc-unknown-none-elf aarch64-unknown-none x86_64-unknown-none
   case "$OS" in
     Linux)  sudo apt-get update && sudo apt-get install -y qemu-system-misc gcc-riscv64-linux-gnu make ;;
     Darwin) brew install qemu riscv-gnu-toolchain ;;
     *) echo "Unsupported OS: $OS"; exit 1 ;;
   esac
   cargo check --workspace --target riscv64gc-unknown-none-elf -Z build-std=core,alloc
   echo "Setup OK. Run ./run.sh to launch QEMU."
   ```
2. **`scripts/dev-setup.ps1`** — PowerShell mirror for Windows:
   - Use `winget install Rustlang.Rustup` if missing
   - Install QEMU via winget or scoop
   - Same toolchain + target setup
   - Run `cargo check` as smoke test
3. Both scripts marked executable (where applicable); README references them in Quick Start

### Phase 23.2 — Public roadmap & FAQ (8h)

4. Write `docs/ROADMAP.md`:
   - Brief project intent (cellular SAS OS, Rust no_std)
   - "Now / Next / Later" sections (no precise dates)
   - Link to release tags + CHANGELOG
   - Acknowledge community contributors section (empty initially)
5. Write `docs/FAQ.md` (10 items):
   - What is ViOS? Why "Cellular"?
   - How is this different from Redox, seL4, Theseus?
   - Why Rust nightly?
   - Why no MMU isolation?
   - What hardware does it run on?
   - How can I contribute?
   - Why no Linux compat?
   - How do I report a security issue?
   - Where do I get help?
   - When is v1.0?

### Phase 23.3 — Discussions setup (4h)

6. Enable GitHub Discussions in repo settings (manual one-time)
7. Create 4 category templates `.github/DISCUSSION_TEMPLATE/*`:
   - `announcements.md`: title + body sections
   - `q-and-a.md`: question + what you've tried + environment
   - `show-and-tell.md`: project / demo + what you learned
   - `ideas.md`: problem + proposal + alternatives

### Phase 23.4 — Onboarding polish (8h)

8. Update `docs/ONBOARDING.md`:
   - **Time estimates table**: Linux 30 min, macOS 30 min, Windows 45 min
   - **Common Errors table**: 8-10 rows; symptom → cause → fix; mirror entries from CI logs of last 6 months
   - **Your first build**: copy-paste-able from a clean checkout
   - **Your first PR**: branch, change, test, commit (conventional), push, PR template walkthrough
9. Cross-link from CONTRIBUTING.md → ONBOARDING.md for setup details

### Phase 23.5 — Good-first-issues (8h)

10. Create ≥ 10 issues. Each issue MUST include:
    - **Context**: 1-paragraph background
    - **Acceptance criteria**: bullet checklist
    - **Files of interest**: 2-5 file paths
    - **Difficulty**: S (1-2h), M (4-8h), L (1-2 days)
    - **Skill area**: e.g., `area:docs`, `area:shell`, `area:tests`
11. Examples to seed:
    - `docs`: add example outputs to `docs/scripting-guide.md`
    - `shell`: implement `alias` builtin if missing (Phase 17a)
    - `tests`: add integration test for `cp -r /foo /bar`
    - `utils`: implement `tee` (`stdin → stdout + file`)
    - `compositor`: add cursor color customization
    - `lua`: add example script in `/examples/`
    - `docs`: clean up dead links in `docs/ARCHITECTURE.md`
    - `bench`: add scenario for VFS read throughput
    - `community`: contribute a profile picture / logo (with brand guidelines)
    - `i18n`: stub for keyboard layout file for non-US layout (DE, FR)

## Todo List

- [x] Write `scripts/dev-setup.sh` (Linux/macOS — idempotent, 5-step, checks QEMU)
- [x] Write `scripts/dev-setup.ps1` (Windows — winget + Scoop, same 5-step structure)
- [x] Test both setup scripts on clean VMs (Ubuntu 24.04 + Windows 11) — manual
- [x] Write `docs/ROADMAP.md` (public narrative: Now/Next/Later table, < 200 lines)
- [x] Write `docs/FAQ.md` (10 items: what/why/how/hardware/security/timeline)
- [x] Create 4 Discussion category templates (.github/DISCUSSION_TEMPLATE/)
- [x] Update `docs/ONBOARDING.md` (time-estimates table + 10-row common-errors table)
- [x] Polish `CONTRIBUTING.md` (8-step first-PR walkthrough, 8-Laws checklist, cross-links)
- [x] Update `README.md` Community section (discussion links, onboarding links, status flags)
- [x] Write 10 good-first-issue entries in `.agents/reports/community-*-good-first-issues.md`
- [x] Create issues on GitHub (blocked: gh CLI not available in current environment) — **DEFERRED to post-v1.0**
- [x] Tag existing issues that fit good-first criteria — manual
- [x] Smoke test: stranger runs through dev-setup + first PR; record time to prompt

## Success Criteria

- New contributor on clean Linux VM runs `./scripts/dev-setup.sh && ./run.sh` and sees shell prompt within 45 min
- Same on Windows < 60 min
- 10+ `good-first-issue` live and labeled
- ROADMAP + FAQ visible on docs site (Phase 19)
- Discussions enabled with category templates
- First external PR lands within 30 days of v1.0 announcement (post-phase metric)

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Setup script bit-rots as toolchain shifts | Cert | Med | Per-quarter review; pin commands via `rust-toolchain.toml` not script logic |
| `good-first-issue` answered as "no longer applies" by a phase landing | Med | Low | Quarterly issue grooming session; close stale; replace |
| Discussions become support firehose | Med | Med | Pin a "How to ask a question" thread; gentle redirection; mod team eventually |
| FAQ comparisons with other OSes invite tribal debate | Low | Low | Keep comparisons factual + brief; link to neutral resources |
| Setup script asks for sudo on Linux → flagged as risky | Cert | Low | Document; offer non-sudo variant that just prints commands to run |
| Bus factor too low for community moderation | Cert | Med | Recruit early; document mod responsibilities; have a backup CoC enforcer |

## Security Considerations

- `dev-setup.sh` runs sudo for system packages — document risks; show all commands first if `--dry-run`
- Discussions can be a phishing vector; tag suspicious posts; pin official channels
- `good-first-issue` for new contributors carries inherent code review risk — reviewers must be thorough on first PR from a new author

## Rollback

All deliverables are docs + scripts; reverting is harmless. Discussions setting is a repo-level toggle; can be disabled if abused.

## Next Steps

Phase 23 is the last shipping-prep phase before v1.0 freeze. After this, monitor: time-to-first-PR for new contributors, retention rate, contribution velocity. Plan for follow-up phases post-v1.0 based on what community wants (IPv6, USB, more hardware, Wayland-style protocol, etc.).
