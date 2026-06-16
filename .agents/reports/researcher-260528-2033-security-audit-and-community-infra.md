# Research Report: Security Audit & Community Infrastructure for ViCell
**Date**: 2026-05-28 | **Scope**: ViCell — no_std nightly, RISC-V 64-bit, ~12,600 LOC, 21 crates

---

## TOPIC 1: SECURITY AUDIT FOR CAPABILITY-BASED OS

### 1.1 STRIDE Threat Matrix — ViCell Specific

ViCell's SAS + LBI model shifts threat surfaces significantly vs. MMU-based OS. Map each STRIDE category to ViCell's architecture:

| STRIDE | Traditional OS | ViCell-Specific Vector | Mitigation |
|--------|---------------|----------------------|------------|
| **Spoofing** | Forged process identity | Cell forging a capability token / cell ID in IPC dispatch | Capability tokens must be unforgeable `Arc<CapToken>` — never pass raw IDs across cell boundary |
| **Tampering** | Write to other process memory | Safe-Rust Cell calling `unsafe` via transitive dependency (unsoundness in dep) | `cargo-geiger` gate on unsafe dep count; Rudra scan on every dep |
| **Repudiation** | No audit log | Kernel IPC log disabled → cell actions untraceable | Kernel-level IPC audit ring buffer; persistent serial log in debug builds |
| **Info Disclosure** | mmap exploitation | Lease/Grant misuse: Cell holds `Lease<T>` beyond revocation window | Runtime lease expiry checks; `// SAFETY:` on every transmit site; Kani verify lease lifetime invariant |
| **DoS** | Fork bomb | Cell monopolizing async executor (no yield point, busy-loop) | Task deadline / yield-point enforcement; Memory Quota Cell (already in testing doc) |
| **EoP** | Kernel exploit | Unsound `unsafe` in kernel or HAL transitively callable from Cell; confused deputy via capability delegation chain | Minimize unsafe TCB (Asterinas model); audit every `unsafe` block with MIRI + Rudra; forbid ambient authority |

**Highest-risk area for ViCell**: Elevation of Privilege via transitive unsoundness. A Cell marked `#![forbid(unsafe_code)]` can still be exploited if a safe API it calls has unsoundness in its `unsafe` implementation (Rudra's primary target). This is the #1 priority audit item.

### 1.2 SAS-Specific Threat Vectors

Hardware MMU is absent between Cells. This means:

1. **Confused Deputy via Capability Delegation** — Cell A delegates a capability to Cell B; Cell B passes it to Cell C without A's knowledge. No hardware enforces delegation chain. Fix: capability tokens must carry delegation depth limit.

2. **Speculative Execution Side-Channels** — All Cells share one address space. A Spectre-style gadget in one Cell leaks data from another Cell's memory via cache timing. Mitigation: RISC-V Sv39 still isolates kernel/user; within-kernel Cell boundaries are software-only and cannot stop timing attacks. **This is an acknowledged fundamental trade-off of SAS — document it explicitly.**

3. **Type Confusion via `unsafe` Transmute** — A malicious Cell dependency could use `std::mem::transmute` on a borrowed pointer to forge a capability. Prevented by `#![forbid(unsafe_code)]` in Cells + Rudra scan of all transitive deps.

4. **TOCTOU on Lease Revocation** — Between lease validity check and actual use, async suspension could change the lease state. Mitigation: lease check-then-use must be in the same sync block, or use atomic revocation flags.

5. **Panic-Unwind Leaking State** — If a Cell panics mid-IPC, partial state can be left in shared kernel structures. ViCell testing doc already calls this out (Fault Injection Cell). Ensure `catch_unwind` on all Cell entry points.

### 1.3 Unsafe Code Audit Toolchain

**Tier 1 — Run in CI (every PR):**

```bash
# Count unsafe blocks per crate — gate on regression
cargo geiger --all-features 2>&1 | tee geiger-report.txt

# Known CVEs in dependencies
cargo audit

# Static analysis — checks for UB patterns in safe Rust wrappers over unsafe
# Install: cargo install cargo-rudra (requires nightly)
cargo rudra 2>&1 | tee rudra-report.txt
```

**Tier 2 — Run weekly / on unsafe changes:**

```bash
# MIRI: interprets MIR, catches UB, works on no_std test harness
# Limitation: cannot run on full kernel binary; run on isolated unit tests
MIRIFLAGS="-Zmiri-disable-isolation" cargo miri test --lib -p libs-types
MIRIFLAGS="-Zmiri-disable-isolation" cargo miri test --lib -p kernel

# MIRI for no_std: set up custom target in miri, use x86_64-unknown-linux-gnu
# for unit test compilation while keeping no_std logic
```

**Tier 3 — Formal verification (quarterly or on critical kernel invariants):**

```bash
# Kani: bounded model checker for Rust, works with no_std
# Install: cargo install --locked kani-verifier && cargo kani setup
cargo kani --harness verify_lease_revocation_invariant
```

**Kani example harness for ViCell:**

```rust
#[cfg(kani)]
mod verification {
    use super::*;
    
    // Prove: a revoked lease cannot be dereferenced
    #[kani::proof]
    fn verify_lease_lifetime() {
        let lease: Lease<u32> = kani::any();
        kani::assume(lease.is_revoked());
        // This should be unreachable — Kani proves it
        assert!(!lease.can_deref());
    }
}
```

**Tool Ranking for ViCell (priority order):**

| Rank | Tool | Purpose | Run Frequency | Limitation |
|------|------|---------|--------------|-----------|
| 1 | `cargo audit` | Known CVEs | Every PR | Only known CVEs |
| 2 | `cargo geiger` | Unsafe count gate | Every PR | Count only, no semantics |
| 3 | `rudra` | Unsoundness patterns | Weekly | Nightly only, some false positives |
| 4 | `MIRI` | UB detection in tests | On unsafe changes | Can't run full kernel binary |
| 5 | `kani` | Formal invariant proofs | Quarterly | Bounded — doesn't prove all inputs |

### 1.4 Fuzzing Strategy

**Problem**: cargo-fuzz / libFuzzer requires std and x86-64. ViCell is no_std + RISC-V target. Cannot fuzz the kernel binary directly with cargo-fuzz.

**Solution — Two-track fuzzing:**

**Track A: Host-side fuzz of parsing/protocol code** (works today)

```bash
# cargo-fuzz works on host (x86-64 linux) for parsing logic extracted to lib
# Add fuzz targets to a std-enabled test crate that exercises the same logic
cargo fuzz add fuzz_elf_parser
cargo fuzz run fuzz_elf_parser -- -max_total_time=3600
```

Fuzz candidates: ELF loader parser (`kernel/src/loader/`), syscall argument decoding, VFS path parsing, config file parsing.

**Track B: LibAFL + QEMU mode for kernel syscall fuzzing** (medium effort)

LibAFL supports `no_std` and has a `libafl_qemu` crate that bridges QEMU user-mode. Run the kernel in QEMU, inject syscall sequences via LibAFL's QEMU harness:

```bash
# LibAFL QEMU mode — fuzzes the running kernel by injecting ecall sequences
# Reference: https://github.com/AFLplusplus/LibAFL
cargo add libafl libafl_qemu --dev
# Write a fuzzer that calls ecall with mutated register values
```

**Track C: Syzkaller-style grammar fuzzing** (longer term)

Write a syscall grammar description for ViCell's `ViSyscall` enum. syzkaller can generate semantically valid syscall sequences. Requires a syz-description file per syscall.

**Recommended start**: Track A (immediate, low cost). Track B after stabilizing syscall interface.

### 1.5 Lessons from Comparable OSes

| OS | Unsafe % of codebase | Approach | Lesson for ViCell |
|----|---------------------|----------|-----------------|
| Tock OS | 93% of crates have unsafe | Scattered, no strict boundary | Anti-pattern — do NOT follow |
| Theseus | 62% | Better but still broad | Borderline acceptable |
| RedLeaf | 32% | Isolated unsafe | Acceptable |
| **Asterinas** | **14% (OSTD only)** | Unsafe confined to one framework crate (OSTD), rest is safe Rust | **Target model for ViCell** |

**ViCell goal**: Confine all `unsafe` to `hal/` + `kernel/src/` core. Every unsafe block has a `// SAFETY:` comment. Target: unsafe-containing files < 20% of total codebase.

Asterinas uses CertiK + Verus for formal verification of page management. ViCell equivalent: use Kani for lease/capability invariants (simpler to adopt than Verus).

### 1.6 Practical 5-Step Security Checklist

```
Step 1 — Unsafe Inventory (Day 1)
  □ Run `cargo geiger --all-features` 
  □ Record baseline unsafe block count per crate
  □ Create policy: kernel/ and hal/ allowed; cells/ must be zero

Step 2 — Dependency Audit (Day 1)
  □ Run `cargo audit`
  □ Run `cargo deny check` (license + duplicate + advisory check)
  □ Pin deps with `Cargo.lock` in version control

Step 3 — Static Analysis (Weekly CI gate)
  □ Add rudra to CI: `cargo rudra` — fail on new unsoundness patterns
  □ Add `cargo geiger` diff check — fail if unsafe count increases

Step 4 — MIRI on Unit Tests (On unsafe change)
  □ Write unit tests for all `unsafe` blocks
  □ Run `cargo miri test` on those tests
  □ Document any MIRI-unverifiable code with rationale

Step 5 — Formal Verification of Critical Invariants (Quarterly)
  □ Identify top 3 kernel invariants (e.g. lease lifetime, cap revocation, memory quota)
  □ Write Kani harnesses for each
  □ Add to CI as slow-path check (`cargo kani --jobs 4`)
```

---

## TOPIC 2: OPEN SOURCE COMMUNITY INFRASTRUCTURE

### 2.1 CONTRIBUTING.md — Required Sections

ViCell already has `ONBOARDING.md` (detailed, well-structured). CONTRIBUTING.md should be the **short entry point** that links to it, not duplicate it. Recommended structure (< 150 lines):

```markdown
# Contributing to ViCell

## Quick Links
- [Code of Conduct](CODE_OF_CONDUCT.md)
- [Full Onboarding Guide](docs/ONBOARDING.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Good First Issues](https://github.com/vi-group/ViCell/labels/good-first-issue)

## Getting Started (5 min)
1. Fork + clone
2. `rustup default nightly && rustup target add riscv64gc-unknown-none-elf`
3. `cargo build --release`
4. See [ONBOARDING.md](docs/ONBOARDING.md) for full setup

## Commit Format
Use conventional commits: `feat(scope): message`
Types: feat | fix | docs | refactor | test | chore

## PR Checklist
- [ ] `cargo check` passes
- [ ] `cargo fmt --all` applied
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo audit` passes
- [ ] No new unsafe in cells/ (see Law 4)
- [ ] `// SAFETY:` on every unsafe block added

## What Needs Contributors
[See labels: good-first-issue, help-wanted, area:hal, area:cells, area:docs]

## Review SLA
Maintainers aim to respond within 7 days.
```

**Missing from current ViCell docs**: CODE_OF_CONDUCT.md — add it (use Contributor Covenant v2.1, 5 min to set up).

### 2.2 Good-First-Issue Labeling Strategy

Kernel projects are intimidating. Structure labels to create a clear on-ramp:

**Difficulty labels:**
- `good-first-issue` — docs fixes, typo, add shell command, add test case (1-3h)
- `help-wanted` — medium complexity, maintainer available for guidance (1-3 days)
- `good-second-issue` — custom label: issues requiring understanding of one subsystem

**Area labels (ViCell-specific):**
- `area:docs` — documentation only (safest for newcomers)
- `area:shell` — cells/apps/shell — self-contained, no unsafe
- `area:hal-traits` — adding traits, no hardware code
- `area:tests` — adding KUnit tests
- `area:cells` — new Cell apps or drivers (no unsafe allowed)
- `area:kernel` — kernel internals (experienced contributors)
- `area:hal-arch` — architecture-specific, RISC-V/ARM (expert only)

**Newcomer-friendly areas ranked:**
1. `area:docs` + `good-first-issue` — zero risk, high value
2. `area:shell` — isolated Cell, pure safe Rust, easy to test
3. `area:tests` — writing KUnit tests teaches the codebase safely
4. `area:cells` — new utility Cell, no unsafe, compile-time guaranteed safe

**Anti-pattern**: Labeling kernel scheduler or HAL issues as `good-first-issue`. These require deep understanding and will frustrate newcomers.

### 2.3 GitHub Discussions vs Issues vs Discord

**Recommendation**: Three-tier model (not either/or).

| Channel | Purpose | ViCell Use Case |
|---------|---------|--------------|
| **GitHub Issues** | Bug reports, feature requests with clear acceptance criteria | "Kernel panics on QEMU virt board when X", "Implement VirtIO block driver" |
| **GitHub Discussions** | Design questions, architecture Q&A, community announcements, FAQ | "Why SAS instead of microkernel with separate address spaces?", "Roadmap discussion" |
| **Discord** | Real-time help, newcomer Q&A, coordination | "I'm stuck setting up QEMU", pair-programming sessions |

**Key insight**: GitHub Discussions is asynchronous, searchable, and linked to the repo — superior to Discord for technical Q&A that should be indexed. Discord is irreplaceable for real-time onboarding but knowledge is lost.

**Rule**: Any Discord answer that takes > 5 min to write should be copied to a GitHub Discussion.

ViCell's existing ONBOARDING.md already points to both — this is correct. Add GitHub Discussions as the primary async channel, Discord as secondary real-time.

### 2.4 Changelog Automation: git-cliff + Conventional Commits

**Recommendation**: git-cliff (written in Rust, language-agnostic binary, highly customizable).

**Setup (< 30 min):**

```bash
# Install
cargo install git-cliff

# Init config in repo root
git cliff --init
# Generates cliff.toml
```

**cliff.toml for ViCell:**

```toml
[changelog]
header = "# ViCell Changelog\n\n"
body = """
## [{{ version | trim_start_matches(pat="v") }}] - {{ timestamp | date(format="%Y-%m-%d") }}
{% for group, commits in commits | group_by(attribute="group") %}
### {{ group | upper_first }}
{% for commit in commits %}
- {{ commit.message | upper_first }} ([{{ commit.id | truncate(length=7, end="") }}]({{ commit.id }}))
{% endfor %}
{% endfor %}
"""
trim = true

[git]
conventional_commits = true
commit_parsers = [
  { message = "^feat", group = "Features" },
  { message = "^fix", group = "Bug Fixes" },
  { message = "^perf", group = "Performance" },
  { message = "^refactor", group = "Refactor" },
  { message = "^docs", group = "Documentation" },
  { message = "^chore", skip = true },
]
```

**GitHub Actions workflow:**

```yaml
# .github/workflows/changelog.yml
on:
  push:
    tags: ['v*']

jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }
      - name: Generate changelog
        uses: orhun/git-cliff-action@v3
        with:
          config: cliff.toml
          args: --verbose --latest
        env:
          OUTPUT: CHANGES.md
      - name: Create GitHub Release
        uses: softprops/action-gh-release@v1
        with:
          body_path: CHANGES.md
```

**Commit type → CHANGELOG section mapping for ViCell:**

| Commit prefix | CHANGELOG group |
|--------------|-----------------|
| `feat(kernel):` | Features |
| `feat(hal):` | Features |
| `fix(...)` | Bug Fixes |
| `perf(...)` | Performance |
| `docs(...)` | Documentation — include in CHANGELOG |
| `chore(...)` | Skip — don't clutter changelog |

### 2.5 Release Tagging Strategy

ViCell uses a hybrid CalVer + SemVer already noted in CHANGELOG.md. Recommended formalization:

**Tag format**: `v0.MINOR.PATCH` for pre-1.0, e.g., `v0.3.0`

**Branching model** (simple, suits small team):
```
main ← always deployable / runnable
↑
feature/xxx branches → PR → main
↑
tags: v0.3.0 on main at milestone
```

**Release cadence for OS project**:
- **Patch** (`v0.x.PATCH`): bug fixes, docs, no behavior change — cut as needed
- **Minor** (`v0.MINOR.0`): new Cell, new HAL arch, new syscall — monthly or per milestone
- **Major** (`v1.0.0`): ABI stability commitment — defer until kernel ABI is stable

**Tag commands:**
```bash
# Create annotated tag (git-cliff reads these)
git tag -a v0.3.0 -m "ViCell Mycelium Alpha - v0.3.0"
git push origin v0.3.0
# GitHub Actions triggers, git-cliff generates release notes automatically
```

**Important**: Use annotated tags (`-a`), not lightweight. git-cliff and cargo-release both require annotated tags for version detection.

### 2.6 Developer Onboarding Assessment

ViCell's existing `docs/ONBOARDING.md` is already strong (240+ lines, well-structured). Gaps and improvements:

**What's good:**
- 20-minute codebase tour
- Syscall trace walkthrough
- Three learning paths (App / Kernel / HAL developer)
- Week 1/2/Month 1 goals

**What's missing:**

1. **"Zero to shell prompt" time estimate** — add: "Expected: 30 minutes on Linux, 45 min on Windows/macOS". Sets realistic expectations.

2. **Troubleshooting table** — QEMU startup failures are the #1 blocker. Add a `## Common Errors` section:
   ```
   | Error | Cause | Fix |
   |-------|-------|-----|
   | `illegal instruction` | Wrong QEMU version | Require qemu >= 8.0 |
   | `no bootable device` | Missing disk.img | Run `python3 create_ramdisk.py` |
   | `nightly not found` | Wrong toolchain | `rustup default nightly` |
   ```

3. **CONTRIBUTING.md missing** — the repo has ONBOARDING.md but no CONTRIBUTING.md at root. GitHub surfaces CONTRIBUTING.md in the "new issue" flow automatically. Create it (< 100 lines, links to ONBOARDING.md).

4. **CODE_OF_CONDUCT.md missing** — required by GitHub for "Insights > Community" badge. Use Contributor Covenant v2.1 verbatim.

5. **Issue template** — add `.github/ISSUE_TEMPLATE/bug_report.md` and `feature_request.md`. Reduces noise from incomplete bug reports.

---

## RECOMMENDATIONS (RANKED)

### Security Audit — Priority Order

1. **Immediate**: Add `cargo audit` + `cargo geiger` to CI. Zero cost, catches known CVEs and unsafe regressions. 2h to set up.
2. **This month**: Add Rudra to weekly CI run. Catches unsoundness patterns cargo audit misses.
3. **This quarter**: Write Kani harnesses for 3 kernel invariants (lease lifetime, capability revocation, memory quota enforcement).
4. **Long-term**: LibAFL + QEMU fuzzing for syscall interface. High value but requires 2-3 weeks of engineering.
5. **Document explicitly**: Spectre/timing side-channel is an acknowledged fundamental limitation of SAS. Write it in `docs/00-context.md` under "Known Trade-offs" to prevent confusion.

### Community Infrastructure — Priority Order

1. **This week**: Create `CONTRIBUTING.md` at repo root (< 100 lines). GitHub auto-surfaces it to contributors.
2. **This week**: Add `CODE_OF_CONDUCT.md` (Contributor Covenant v2.1 — verbatim copy, 5 min).
3. **This month**: Set up git-cliff + cliff.toml. Automate CHANGELOG on tag push.
4. **This month**: Create GitHub issue templates. Reduce noise, improve bug report quality.
5. **Next milestone**: Triage 5-10 issues as `good-first-issue` in `area:docs` and `area:shell`. Without labeled issues, newcomers have no entry point.

---

## UNRESOLVED QUESTIONS

1. **MIRI on no_std kernel**: MIRI requires a host target to run. ViCell unit tests likely need a `[dev-dependencies]` std-enabled shim or feature flag to run under MIRI. The exact approach (conditional compilation vs. separate test crate) is unverified — needs a spike.

2. **Kani + no_std**: The Merlin OS case confirms Kani works with no_std kernels under RISC-V constraints, but ViCell uses alloc + custom allocator. Whether Kani can model the custom allocator's invariants is untested.

3. **LibAFL QEMU mode maturity**: LibAFL's QEMU bridge (`libafl_qemu`) is documented as stable for user-mode ELF fuzzing but kernel-mode syscall fuzzing requires custom harness work. Effort estimate is uncertain (2 weeks to 2 months depending on QEMU integration complexity).

4. **Rudra maintenance status**: Rudra's last upstream commit was 2022. It still runs on recent nightly but its false-positive rate on 2025 Rust idioms is undocumented. Treat as supplementary, not definitive.

---

## SOURCES

- [cargo-geiger GitHub](https://github.com/geiger-rs/cargo-geiger)
- [RUDRA paper (SOSP 2021)](https://www.micahlerner.com/assets/papers/rudra.pdf)
- [Kani Rust Verifier](https://model-checking.github.io/kani/)
- [Asterinas kernel memory safety blog (2025)](https://asterinas.github.io/2025/06/04/kernel-memory-safety-mission-accomplished.html)
- [Asterinas formal verification post (2025)](https://asterinas.github.io/2025/02/13/towards-practical-formal-verification-for-a-general-purpose-os-in-rust.html)
- [Asterinas ATC'25 paper](https://arxiv.org/abs/2506.03876)
- [LibAFL GitHub](https://github.com/AFLplusplus/LibAFL)
- [cargo-fuzz Rust Fuzz Book](https://rust-fuzz.github.io/book/cargo-fuzz.html)
- [git-cliff official site](https://git-cliff.org/)
- [Orhun's automated Rust releases blog](https://blog.orhun.dev/automated-rust-releases/)
- [GitHub Discussions vs Issues — DEV Community](https://dev.to/mishmanners/github-issues-or-github-discussions-whats-the-difference-and-when-should-you-use-each-one-4lhd)
- [Open Source Labeling Best Practices — Dosu](https://dosu.dev/blog/open-source-labeling-best-practices)
- [Capability-Based Security Model — Medium](https://medium.com/@sohail_saifii/the-capability-based-security-model-that-makes-privilege-escalation-impossible-8231d679b972)
- [Rust Auditing Tools 2025 — Markaicode](https://markaicode.com/rust-auditing-tools-2025-automated-security-scanning/)
- [Merlin OS formal verification with Kani](https://paolozaino.wordpress.com/2025/08/08/merlin-os-building-trust-in-risc-os-merlin-with-formal-verification-methods/comment-page-1/)
