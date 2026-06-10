"""Write Phase 19 documentation automation files."""
import os

base = "d:/ViCell"

docs_yml = """name: Docs

on:
  push:
    branches: [main]
    paths: ['docs/**', 'libs/**', 'kernel/**', 'cells/**', 'hal/**']

env:
  CARGO_INCREMENTAL: 0
  CARGO_TERM_COLOR: always

jobs:
  rustdoc:
    name: Build and deploy rustdoc
    runs-on: ubuntu-24.04
    permissions:
      contents: write
    steps:
      - uses: actions/checkout@v4
      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@master
        with:
          toolchain: nightly-2026-05-01
          components: rust-src
      - uses: Swatinem/rust-cache@v2
        with: { cache-on-failure: true }
      - name: Build rustdoc
        run: >
          cargo doc --workspace --no-deps
          --target riscv64gc-unknown-none-elf
          -Z build-std=core,alloc
      - name: Generate llms.txt
        run: bash scripts/gen-llms-txt.sh
      - name: Deploy to GitHub Pages
        uses: peaceiris/actions-gh-pages@v4
        with:
          github_token: ${{ secrets.GITHUB_TOKEN }}
          publish_dir: target/riscv64gc-unknown-none-elf/doc
          destination_dir: api
"""

release_yml = """name: Release

on:
  push:
    tags: ['v*']

env:
  CARGO_INCREMENTAL: 0
  CARGO_TERM_COLOR: always

jobs:
  release:
    name: Create GitHub Release
    runs-on: ubuntu-24.04
    permissions:
      contents: write
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }
      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@master
        with:
          toolchain: nightly-2026-05-01
          components: rust-src
      - uses: Swatinem/rust-cache@v2
        with: { cache-on-failure: true }
      - name: Install git-cliff
        run: cargo install --locked git-cliff
      - name: Generate CHANGELOG for this tag
        run: git-cliff --current > RELEASE_NOTES.md
      - name: Update full CHANGELOG.md
        run: |
          git-cliff --output CHANGELOG.md
          git config user.name "ViCell Team"
          git config user.email "ci@ViCell"
          git add CHANGELOG.md
          git commit -m "chore(release): update CHANGELOG" || true
      - name: Build kernel (RV64)
        run: >
          cargo build --release
          --target riscv64gc-unknown-none-elf
          -Z build-std=core,alloc
      - name: Create GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          body_path: RELEASE_NOTES.md
          files: target/riscv64gc-unknown-none-elf/release/vicell-kernel
"""

cliff_toml = """[changelog]
header = "# Changelog\\n\\nAll notable changes to ViCell.\\n"
body = \"\"\"
{% if version %}## [{{ version | trim_start_matches(pat="v") }}] - {{ timestamp | date(format="%Y-%m-%d") }}
{% else %}## [unreleased]
{% endif %}\\
{% for group, commits in commits | group_by(attribute="group") %}
### {{ group | striptags | trim | upper_first }}
{% for commit in commits %}
- {% if commit.scope %}**{{ commit.scope }}**: {% endif %}{{ commit.message | upper_first }}{% if commit.breaking %} [**BREAKING**]{% endif %}
{% endfor %}
{% endfor %}
\"\"\"
trim = true
footer = ""

[git]
conventional_commits = true
filter_unconventional = true
commit_parsers = [
    { message = "^feat",     group = "Features"       },
    { message = "^fix",      group = "Bug Fixes"      },
    { message = "^perf",     group = "Performance"    },
    { message = "^docs",     group = "Documentation"  },
    { message = "^test",     group = "Testing"        },
    { message = "^security", group = "Security"       },
    { message = "^chore",    group = "Maintenance"    },
    { message = "^build",    group = "Build"          },
]
filter_commits = false
tag_pattern = "v[0-9]*"
"""

contributing_md = """# Contributing to ViCell

Welcome! ViCell is a `no_std` Rust OS with a Cellular Single Address Space architecture.

## Quick Start

### Prerequisites
- Rust nightly (pinned in `rust-toolchain.toml`)
- QEMU with RISC-V support: `qemu-system-riscv64`
- RISC-V cross-compiler: `riscv-none-elf-gcc` (xpack release)
- Python 3.10+ (for disk image tooling)

### Build
```bash
cargo build --release --target riscv64gc-unknown-none-elf -Z build-std=core,alloc
```

### Run
```powershell
./run.ps1
```
or
```bash
bash scripts/run-aarch64.sh  # AArch64
bash scripts/run-x86-64.sh  # x86_64
```

## Code Standards

1. **Law 4 (unsafe):** Every `unsafe` block requires `// SAFETY:` explaining the invariant.
2. **Law 5 (module style):** Use `foo.rs` + `foo/` — never `mod.rs`.
3. **Law 6 (naming):** Public traits get the `Vi` prefix (`ViFileSystem`, `ViDriver`).
4. **YAGNI / KISS / DRY** — see `docs/code-standards.md`.

## Submitting a PR

1. Fork and create a feature branch.
2. Write code; run `cargo check --workspace` (must be zero warnings).
3. Add a test or document why one isn't needed.
4. Submit PR — the CI template will guide you through the checklist.

## Where to Start

Look for issues labelled [`good-first-issue`](../../issues?q=label%3Agood-first-issue).

## Questions

Open a Discussion or join the project chat (see README for links).
"""

changelog_seed = """# Changelog

## [0.2.0] — 2026-05-28

### Features
- RV64 HAL: SV39 paging, PLIC, SBI, UART
- Kernel ELF loader and basic scheduler
- Shell REPL with history, arrow keys
- RamFS VFS serving `/bin/` from embedded ELFs
- Config KV store cell
- POSIX C Library shim (PR #6)
- Async kernel executor (PR #4)
- AArch64 HAL: boot, GIC-400, Generic Timer, PL011, paging
- x86_64 HAL: GDT/IDT, LAPIC, COM1, PML4 paging
- External ELF loading via SpawnFromPath syscall + bootstrap disk table
- Capability-based FileHandle IPC (CapId, OpenCap, ReadCap, CloseCap)
- Lua 5.4 C binding with ViCell glue layer
- STRIDE security model documentation + cargo-geiger CI gate
"""

llms_txt = """# ViCell

> ViCell is a Rust no_std OS using Cellular Single Address Space + Language-Based Isolation.

## Core

- [Architecture Overview](docs/ARCHITECTURE.md): System design and Cell lifecycle
- [Coding Guide](docs/CODING_GUIDE.md): How to write ViCell code
- [Patterns](docs/PATTERNS.md): Common design patterns
- [API Reference](docs/API.md): Public API for Cells and the kernel
- [Onboarding](docs/ONBOARDING.md): Getting started

## Technical Design

- [Memory Model](docs/02-memory.md): SAS, frame allocator, capability system
- [Hardware Abstraction](docs/04-hardware.md): Multi-arch HAL
- [VFS Architecture](docs/09-vfs.md): Virtual filesystem design
- [Testing Strategy](docs/10-testing.md): Test pyramid and QEMU harness

## Security

- [Security Model](docs/security-model.md): STRIDE threat model
- [Code Standards](docs/code-standards.md): Unsafe management

## Optional

- [System Architecture](docs/system-architecture.md): VirtIO, interrupts, paging
- [Project Roadmap](docs/development-roadmap.md): v0.2 to v1.0 milestones
- [Changelog](CHANGELOG.md): Version history
"""

gen_llms_sh = """#!/usr/bin/env bash
# Generate llms.txt from the docs/ directory index.
set -euo pipefail
python3 -c "
import os, re
lines = ['# ViCell', '', '> ViCell is a Rust no_std OS using Cellular SAS + LBI.', '', '## Docs', '']
for root, dirs, files in os.walk('docs'):
    dirs.sort()
    for f in sorted(files):
        if f.endswith('.md'):
            path = os.path.join(root, f)
            rel  = path.replace(chr(92), '/')
            with open(path, encoding='utf-8', errors='ignore') as fh:
                first = fh.readline().strip().lstrip('# ')
            lines.append(f'- [{first}]({rel})')
print('\\n'.join(lines))
" > llms.txt
echo "llms.txt updated ($(wc -l < llms.txt) lines)"
"""

output = {
    ".github/workflows/docs.yml": docs_yml,
    ".github/workflows/release.yml": release_yml,
    "cliff.toml": cliff_toml,
    "CONTRIBUTING.md": contributing_md,
    "CHANGELOG.md": changelog_seed,
    "llms.txt": llms_txt,
    "scripts/gen-llms-txt.sh": gen_llms_sh,
}

for relpath, content in output.items():
    full = f"{base}/{relpath}"
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "w", encoding="utf-8", newline="\n") as f:
        f.write(content)
    print(f"  wrote {relpath}")

print("Done.")
