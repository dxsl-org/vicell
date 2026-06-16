# ViCell: Security Audit & Community Infrastructure Research
**Date**: 2026-05-28 | **Status**: Final

---

## TOPIC 1: SECURITY AUDIT FOR CAPABILITY-BASED OS

### 1.1 STRIDE for ViCell Cellular/SAS Model

ViCell's threat model differs fundamentally from POSIX: no process boundaries, all code in Ring 0, isolation enforced by Rust's type system + ZST capability tokens + Ed25519-signed Cells.

| STRIDE Category | ViCell-Specific Threat | Mitigation Already In Place | Gap |
|---|---|---|---|
| **Spoofing** | Cell impersonation — unsigned Cell binary loaded as trusted | Ed25519 signature on each Cell ELF | Verify signature check happens BEFORE relocation, not after |
| **Tampering** | Runtime symbol table poisoning — malicious Cell replaces a function pointer in Global Symbol Table | Lock-free hash table (immutable after registration?) | Need read-only lock on symbol table after boot; audit `cell/registry` |
| **Repudiation** | Capability token issued but no audit trail of who called what | Not present | Implement capability use log in debug builds |
| **Info Disclosure** | Cell A reads Cell B's heap via pointer arithmetic | Rust borrow checker at compile time | Only protects safe Rust; any `unsafe` block in HAL/kernel is attack surface |
| **DoS** | Fault Injection Cell already planned (Memory Quota, Watchdog) | Memory Quota + Watchdog in testing plan | Need rate-limiting on IPC/symbol lookups; stack exhaustion via deep async chains |
| **Elevation of Privilege** | A Cell obtaining a capability ZST it was never issued (confused deputy) | ZST tokens not Copy/Clone by design | Audit all `unsafe transmute` paths; check if Box<dyn> downcasting leaks capability types |

**Highest-priority STRIDE item for ViCell**: Elevation of Privilege via capability token smuggling through `unsafe` transmute or raw pointer reinterpretation. seL4's formal proof focused on exactly this class of violations.

### 1.2 SAS-Specific Threat Vectors (No Hardware MMU Isolation)

These threats are unique to ViCell's architecture and don't apply to conventional OSes:

1. **Type confusion via unsafe transmute** — TYPEPULSE (USENIX Security 2025) identified that generic-to-u8 reinterpretation in unsafe Rust is the dominant type confusion source. A Cell that can transmute a `Box<dyn ViDriver>` into raw bytes then cast to another trait object bypasses all ZST capability enforcement.

2. **Stale vtable after Cell unload** — When a Cell is unloaded, any `Arc<dyn Trait>` held by other Cells becomes a dangling vtable pointer. Rust's Drop doesn't help if the Arc refcount is bypassed via unsafe. This is use-after-free in disguise; Theseus OS calls this the "MappedPages hazard."

3. **Symbol table TOCTOU** — Between the time a Cell registers a symbol and when another Cell uses it, an adversarial Cell could unload-then-reload with a different implementation at the same symbol name. Requires atomic snapshot semantics on the registry.

4. **Async lifetime escape** — An async task holding a raw pointer across an `.await` point. The owned-buffer rule (Law 2) mitigates this for public APIs but internal kernel async code may still use `unsafe` futures with raw lifetime coercions.

5. **Capability token copy via `mem::forget` + clone workaround** — ZSTs are zero-size but if a Cell uses `ptr::read` on a ZST address, it creates a second instance. Not exploitable in safe Rust, but any unsafe block near capability ZSTs must be audited.

**Reference**: CheriOS (Cambridge, TR-961) and Theseus OS book both document that SAS safety degrades entirely to language-level guarantees — a single unsound `unsafe` block can break the entire system.

### 1.3 Fuzzing Strategy

**Recommended stack (ranked)**:

**1. LibAFL in QEMU mode (primary)**
- `libafl` + `libafl_qemu` crates; supports `no_std`, scales across cores
- Run ViCell in QEMU, inject malformed syscall/IPC sequences at the Cell API boundary
- LibAFL supports coverage-guided fuzzing of the kernel without modifying kernel source
- Scales linearly across cores via LLMP (Low-Level Message Passing)
- Adoption risk: LOW — actively maintained by AFLplusplus team, production use at Google

**2. cargo-fuzz / libFuzzer (secondary, for unit-level)**
- Use for fuzzing individual kernel subsystems that can be extracted to host (e.g., the ELF loader, symbol table lookup, capability token parsing)
- Limitation: requires `std` harness wrapper; ViCell is `no_std` so you need a thin host shim
- `cargo fuzz run <target>` is the simplest entry point

**3. KernMiri (exploratory, not production-ready)**
- Asterinas built KernMiri (1,200 LOC extension to Miri) to detect UB in OS-specific memory ops
- Cannot run in CI yet due to speed; useful for spot-checking critical `unsafe` blocks
- Adoption risk: HIGH — not a standalone released tool; must fork from Asterinas repo

**Fuzzing targets in priority order**:
1. Cell ELF signature verification path
2. Symbol table registration/lookup under concurrent load
3. Capability token issuance in `init()` path
4. IPC message deserialization (if any)

### 1.4 Unsafe Code Audit Tools

| Tool | Purpose | Verdict for ViCell |
|---|---|---|
| `cargo-geiger` | Reports unsafe block counts per crate | Run first; gives baseline. `cargo install cargo-geiger` |
| Rudra | Detects send/sync variance bugs, panic safety, use-after-free patterns in unsafe Rust | High value for HAL/kernel crates; run on `kernel/` and `hal/` only |
| Miri | UB detection at runtime, interpreter-based | Can't run ViCell binary directly; use for isolated host-side tests of kernel algorithms |
| KernMiri | Miri extended for OS physical memory / paging | Aspirational; copy approach from Asterinas when HAL is stable |
| TYPEPULSE | Detects type confusion in unsafe generic→concrete casts | Academic tool (USENIX 2025); not yet a cargo plugin |

**Immediate actionable audit workflow**:
```
cargo geiger 2>&1 | grep -v "0 unsafe" > unsafe_inventory.txt
# Triage by: HAL (expected) → kernel (scrutinize) → cells (should be zero)
```

Cells must produce zero unsafe blocks (Law 4). Any `geiger` hit in `cells/` is a policy violation.

### 1.5 Formal Verification: Kani

**Kani Rust Verifier** (AWS, actively maintained, cargo-integrated) is the right tool for ViCell invariant proofs:

```toml
# Cargo.toml
[dev-dependencies]
kani = "0.x"
```

```rust
#[cfg(kani)]
#[kani::proof]
fn verify_capability_not_copyable() {
    let cap: RebootCap = kani::any();
    // Prove that no safe code can duplicate the ZST token
}
```

Kani uses bounded model checking (CBMC backend). It can prove:
- Capability tokens cannot be duplicated through safe API
- Symbol table lookup is free of integer overflow
- ELF segment mapping stays within allocated range

**Limitation**: Kani cannot currently verify `unsafe` blocks with raw pointer aliasing across async boundaries. For that, the approach is: minimize unsafe surface (cargo-geiger), then manually review residual unsafe.

**Comparison with seL4 approach**: seL4 used Isabelle/HOL for full functional correctness — 200k LOC proof for ~9k LOC kernel. That's out of scope for ViCell at v0.x. Kani proofs on 5–10 critical invariants is the practical 80/20 choice.

### 1.6 Lessons from Reference OS Projects

| OS | Security Approach | Applicable to ViCell |
|---|---|---|
| **Tock** | Rust capsule system + MPU for hardware isolation; OSFC 2024 talk on formal verification of isolation guarantees | Tock's capsule permission model mirrors ViCell ZST capabilities; study their `AppSlice` ownership model |
| **Theseus** | All code Ring 0 SAS + safe Rust; MappedPages abstraction owns physical frames exclusively | Most architecturally similar to ViCell; their "intralingual OS" design paper is essential reading |
| **Asterinas** | Framekernel: tiny unsafe TCB (14% of code) + all services in safe Rust; KernMiri for UB; Verus for proofs | Aspire to their TCB ratio; ViCell should track `unsafe` LOC % as a metric |
| **seL4** | Isabelle/HOL full proof; capability access control formally verified | Too heavyweight for v0.x; adopt the *design pattern* (minimize TCB, explicit capability derivation tree) |

---

## TOPIC 2: OPEN SOURCE COMMUNITY INFRASTRUCTURE

### 2.1 CONTRIBUTING.md Structure (OS Project Specific)

Ranked sections by contributor value:

```markdown
# Contributing to ViCell

## Quick Start (< 5 min to first build)
- Prerequisites: Rust nightly, QEMU, ...
- `./run.ps1` to boot in QEMU

## How to Find Work
- Beginner: issues labeled `good-first-issue`
- Intermediate: `help-wanted`
- Architecture: read docs/01-core.md first

## Submission Process
1. Fork → branch → PR
2. Run `cargo clippy -- -D warnings` before push
3. One feature/fix per PR

## What We Don't Accept (saves time)
- `mod.rs` files (Law 5)
- `unsafe` in cells/ (Law 4)
- PRs without issue reference
```

**Critical for OS projects**: Include a "What Won't Merge" section. OS newcomers often submit superficially correct code that violates deep architectural invariants (e.g., using process-model thinking in a SAS system). State rejections explicitly to save review cycles.

RavynOS and matrix-org both use this pattern — the "don't bother with X" list prevents wasted effort more than any positive guide.

### 2.2 Issue Labeling Strategy

Recommended label taxonomy:

```
# Entry level
good-first-issue     — Self-contained, < 50 LOC change, no arch knowledge needed
help-wanted          — Needs effort but not deep expertise
docs                 — Pure documentation, zero risk

# Domain
kernel               — kernel/src/** changes
hal                  — hal/arch/** or hal/traits/**
cell                 — cells/apps, cells/drivers, cells/services
testing              — test infrastructure

# Priority
p0-critical          — Boot broken / regression
p1-high              — Feature blocking
p2-normal            — Standard work
p3-low               — Nice to have

# State
needs-design         — Requires architecture discussion first
needs-repro          — Bug without reproduction steps
blocked              — Waiting on another issue
```

**Key insight from research**: Projects that label ~25% of issues as `good-first-issue` see 13% more new contributors. For ViCell specifically, good candidates are:
- Documentation improvements (docs/ directory)
- Adding `kani` proof harnesses for existing invariants
- Porting a new architecture's UART impl to the `ViUart` trait
- Adding `cargo-geiger` CI step

### 2.3 Communication Platform Decision

**Recommendation: GitHub Discussions as primary + Discord as secondary**

| Platform | Strength | Weakness | Verdict |
|---|---|---|---|
| GitHub Discussions | Searchable, indexed by Google, persists alongside code, no login required to read | Async-only, no real-time feel | **Primary** — architecture decisions, design Q&A, release notes |
| Discord | Real-time, lowers barrier for quick questions, good for voice debugging sessions | Not searchable, answers lost, proprietary | **Secondary** — onboarding help, contributor chat |
| Issues | Structured, trackable, linked to code | Too formal for discussion | Bug reports + feature requests only |

**Rationale**: OS newcomers will Google "ViCell how to implement driver" — GitHub Discussions answers are indexed, Discord answers are not. Architectural decisions must be preserved. Discord is for the human warmth that keeps contributors engaged.

Avoid Matrix/IRC for a small project: fragmentation risk outweighs openness benefit until community exceeds ~200 active members.

### 2.4 Changelog Automation

**Recommended: git-cliff (written in Rust, maintained by orhun)**

```toml
# cliff.toml
[changelog]
header = "# Changelog\n\n"
body = """
{% for group, commits in commits | group_by(attribute="group") %}
### {{ group | upper_first }}
{% for commit in commits %}
- {{ commit.message | upper_first }} ([{{ commit.id | truncate(length=7, end="") }}]({{ commit.github.pr_url }}))
{% endfor %}
{% endfor %}
"""

[git]
conventional_commits = true
commit_parsers = [
  { message = "^feat", group = "Features" },
  { message = "^fix", group = "Bug Fixes" },
  { message = "^perf", group = "Performance" },
  { message = "^refactor", group = "Refactoring" },
  { message = "^docs", group = "Documentation" },
  { message = "^chore\\(release\\)", skip = true },
]
```

```yaml
# .github/workflows/release.yml — trigger on tag push
- name: Generate Changelog
  run: git-cliff --current --output CHANGELOG.md
```

git-cliff is written in Rust (uses git2 + tera), actively maintained, supports GitHub PR links, and generates Keep-a-Changelog format. No Node.js dependency, fits ViCell's Rust-first toolchain.

Alternative `release-plz` (also Rust) auto-bumps Cargo.toml versions and opens release PRs — worth evaluating when ViCell reaches v1.0.

### 2.5 Release Tagging Strategy

**ViCell is pre-1.0; apply Cargo's "left-shifted" semver rules:**

```
v0.MINOR.PATCH

v0.MINOR bump   → Breaking change (new required trait method, removed capability type)
v0.x.PATCH bump → Additive fix, no ABI change

Examples:
v0.2.0 → v0.2.1  : Bug fix in ELF loader
v0.2.1 → v0.3.0  : New ViFileSystem trait method added (breaking for Cell authors)
```

**Git tag convention**:
```bash
git tag -a v0.3.0 -m "Release v0.3.0: VFS stabilization"
git push origin v0.3.0
```

**GitHub Release automation** (via git-cliff + gh CLI):
```bash
git-cliff --current > NOTES.md
gh release create v0.3.0 --notes-file NOTES.md --title "ViCell v0.3.0"
```

**Do not use `-rc` suffixes until approaching v1.0**. For early OS development, even numbered releases are unstable by definition; the version number communicates interface stability, not production readiness.

### 2.6 Developer Onboarding: First-Contribution UX

What makes OS newcomers succeed (drawn from Asterinas, Tock, and RavynOS patterns):

**1. Zero-friction dev environment**
- Asterinas ships OSDK (`cargo osdk run`) that eliminates manual QEMU configuration
- ViCell equivalent: ensure `./run.ps1` works on a fresh clone; document all prereqs in ONBOARDING.md with exact version numbers
- Add a `Dockerfile` or `devcontainer.json` for contributors who don't want to install RISC-V cross toolchain locally

**2. Guided first issue design**
- `good-first-issue` must be self-contained: link to the specific file, describe the exact change needed, include acceptance criteria
- Bad: "Improve error handling in kernel" — Good: "Add `ViError::InvalidCapability` variant to `libs/types/src/error.rs` and update the 3 match sites listed below"
- Target < 2 hours for a `good-first-issue` resolution

**3. Architecture glossary**
- ViCell uses non-standard terminology (Cell, Nano Kernel, SAS, LBI) that differs from every other OS project
- A `GLOSSARY.md` or inline in ONBOARDING.md saves every newcomer 30 minutes of confusion
- Map ViCell terms to familiar equivalents: "Cell = process (but shares address space)" etc.

**4. Mentored PR pathway**
- Tag newcomer PRs with `first-contribution`; assign a maintainer reviewer within 48h
- Linux Kernel Mentorship Program model: structured mentorship with monthly checkins — too heavy for v0.x ViCell; lighter version is a "PR buddy" volunteer list in CONTRIBUTING.md

**5. Runnable examples before kernel hacking**
- New contributors should be able to build a trivial Cell ("hello world" driver) before touching kernel code
- This validates their toolchain and teaches the Cell model without risk of breaking the kernel

---

## RANKED RECOMMENDATIONS

### Topic 1 (Security) — Priority Order

1. **Run cargo-geiger now** — establishes unsafe inventory baseline; zero-cost
2. **Enforce `#![forbid(unsafe_code)]` in all `cells/`** with CI check — prevents regression
3. **Add LibAFL QEMU fuzzer targeting Cell API boundary** — highest ROI for finding real bugs
4. **Write 3–5 Kani proof harnesses** for capability ZST non-copyability and symbol table bounds
5. **Audit all `unsafe` blocks in `kernel/src/cell/`** manually using Rudra output as a guide
6. **KernMiri / TYPEPULSE** — defer to v0.4+; tooling is not yet stable for general adoption

### Topic 2 (Community) — Priority Order

1. **CONTRIBUTING.md with "What Won't Merge" section** — immediate contribution quality filter
2. **Label taxonomy + 5–10 real `good-first-issue`s** — prerequisite for community growth
3. **git-cliff + conventional commits** — automates changelog, zero ongoing cost
4. **GitHub Discussions (open now) + Discord (when > 50 active contributors)**
5. **GLOSSARY.md** — uniquely high value for ViCell because of non-standard terminology
6. **Devcontainer / Dockerfile** — defer until Phase 2 boot is stable

---

## TRADE-OFF MATRIX

| Tool / Decision | Value | Effort | Timing |
|---|---|---|---|
| cargo-geiger | High | Trivial | Now |
| LibAFL fuzzing | High | Medium (2–3 days setup) | After Phase 1 boot |
| Kani proofs | Medium | Medium | After API stabilizes |
| Rudra | Medium | Low | Now (one-shot audit) |
| KernMiri | High future | High (fork+maintain) | v0.4+ |
| git-cliff | High | Low | Now |
| GitHub Discussions | High | Trivial | Now |
| Discord | Medium | Low | After first external contributor |
| Devcontainer | Medium | Medium | Phase 2 |

---

## UNRESOLVED QUESTIONS

1. Are ViCell capability ZST tokens currently `!Copy + !Clone` enforced at the type level, or only by convention? If by convention, a `ptr::read` on ZST address bypasses it without triggering unsafe warnings in stable Rust — needs code audit.

2. Does the Global Symbol Table allow overwriting an existing symbol registration? If yes, that's the TOCTOU attack vector described above.

3. Is the Ed25519 signature check performed before or after ELF relocation? Post-relocation verification is too late — a crafted ELF could exploit the loader before verification completes.

4. What is the current unsafe LOC count? cargo-geiger will answer this; the answer determines audit scope.

5. For the community: is there a target contributor count / timeline? Affects whether Discord is premature (adds maintenance burden before network effects kick in).

---

## SOURCES

- [Theseus OS Safe-Language OS Principles](https://www.theseus-os.com/Theseus/book/design/idea.html)
- [Asterinas: Framekernel with Sound TCB (arXiv 2506.03876)](https://arxiv.org/html/2506.03876v1)
- [Asterinas Kernel Memory Safety: Mission Accomplished (2025)](https://asterinas.github.io/2025/06/04/kernel-memory-safety-mission-accomplished.html)
- [TYPEPULSE: Type Confusion in Rust (USENIX Security 2025)](https://arxiv.org/pdf/2502.03271)
- [LibAFL Advanced Fuzzing Library (no_std support)](https://github.com/AFLplusplus/LibAFL)
- [Kani Rust Verifier](https://model-checking.github.io/kani/)
- [cargo-geiger: Unsafe detection](https://github.com/geiger-rs/cargo-geiger)
- [seL4 Formal Verification](https://sel4.systems/Verification/)
- [Tock OS OSFC 2024: Provable Security](https://talks.osfc.io/osfc-2024/talk/WGG9AH/)
- [CheriOS: Untrusted SAS Design (Cambridge TR-961)](https://www.cl.cam.ac.uk/techreports/UCAM-CL-TR-961.pdf)
- [git-cliff Changelog Generator](https://git-cliff.org/)
- [Confused Deputy Problem - Wikipedia](https://en.wikipedia.org/wiki/Confused_deputy_problem)
- [GitHub Discussions vs Community Platforms](https://medium.com/@0xbharath/on-choosing-a-platform-for-an-open-source-community-d26bab4d9d8c)
- [Rust Auditing Tools 2025](https://markaicode.com/rust-auditing-tools-2025-automated-security-scanning/)
- [Good First Issues Impact Research (RecGFI)](https://hehao98.github.io/files/2022-recgfi.pdf)
