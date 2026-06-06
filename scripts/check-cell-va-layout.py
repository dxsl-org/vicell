#!/usr/bin/env python3
"""Build-time guard: every Cell's linker base must be off-MMIO, below RAM, and unique.

ViCell is a Single Address Space (SAS): all Cells share ONE page table. A Cell whose
fixed load VA collides with a kernel MMIO identity map (CLINT/PLIC/UART) or with another
live Cell silently clobbers PTEs (kernel/src/memory/paging.rs maps MMIO identity). This
exact class bit us twice: vfs sat inside CLINT and bench/lua/micropython inside PLIC,
silently corrupting interrupt-controller mappings; an even older default put two cells at
the same base. The kernel's load-time overwrite-guard (loader/elf.rs) catches it at boot,
but this script catches it at BUILD — faster feedback, and it runs in CI before merge.

This is a STATIC check: it parses the linker scripts only (no build needed). It catches
base-inside-MMIO and duplicate-base collisions. Size-overlap (a Cell's image extending
into a neighbour) is still caught at load by the runtime overwrite-guard.

Exit 0 = all clear; exit 1 = a violation (prints the offending Cell + how to fix).

IMPORTANT: the MMIO windows below MUST stay in sync with kernel/src/memory/paging.rs.
"""
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# Kernel MMIO identity maps — keep in sync with kernel/src/memory/paging.rs (~L140-148).
# (name, start_inclusive, end_exclusive)
MMIO_WINDOWS = [
    ("CLINT", 0x0200_0000, 0x0201_0000),
    ("PLIC", 0x0C00_0000, 0x1000_0000),
    ("UART/VirtIO", 0x1000_0000, 0x1001_0000),
]
RAM_BASE = 0x8000_0000  # physical RAM / kernel base on QEMU virt — cells must load below
MIN_BASE = 0x0001_0000  # leave the lowest 64 KiB unmapped (null-deref trap region)

LD_LITERAL = re.compile(r"-T\s*([A-Za-z0-9_./\\-]+\.ld)")
LD_PUSH = re.compile(r'\.push\("([^"]+\.ld)"\)')
BASE_RE = re.compile(r"\.\s*=\s*(0x[0-9A-Fa-f_]+)\s*;")


def resolve_ld_for(build_rs: Path) -> Path | None:
    """Find the linker script a Cell's build.rs feeds to the linker via `-T`.

    Two patterns in this repo: a literal `-Tcells/.../x.ld` arg, or shell's
    pop()+push("x.ld") that walks up from the crate dir. Returns None if neither.
    """
    text = build_rs.read_text(encoding="utf-8", errors="replace")
    m = LD_LITERAL.search(text)
    if m:
        return (REPO / m.group(1)).resolve()
    m = LD_PUSH.search(text)
    if m:
        base = build_rs.parent  # crate dir
        base = base.parent if ".pop()" in text else base
        return (base / m.group(1)).resolve()
    return None


def parse_base(ld: Path) -> int | None:
    """Extract the first `. = 0x...;` location-counter assignment (the load base)."""
    for line in ld.read_text(encoding="utf-8", errors="replace").splitlines():
        m = BASE_RE.search(line)
        if m:
            return int(m.group(1).replace("_", ""), 16)
    return None


def in_mmio(addr: int):
    for name, lo, hi in MMIO_WINDOWS:
        if lo <= addr < hi:
            return name
    return None


def main() -> int:
    cells = []  # (name, base, ld_relpath)
    errors = []

    for build_rs in sorted((REPO / "cells").rglob("build.rs")):
        if "vendor" in build_rs.parts:
            continue
        ld = resolve_ld_for(build_rs)
        if ld is None:
            continue  # cell doesn't use a custom linker script — skip
        name = build_rs.parent.name
        if not ld.exists():
            errors.append(f"{name}: build.rs references missing linker script {ld}")
            continue
        base = parse_base(ld)
        if base is None:
            errors.append(f"{name}: no `. = 0x...;` base found in {ld.name}")
            continue
        rel = ld.relative_to(REPO).as_posix()
        cells.append((name, base, rel))

        window = in_mmio(base)
        if window is not None:
            errors.append(
                f"{name}: base 0x{base:08X} is INSIDE the {window} MMIO window "
                f"({rel}) — loading this cell clobbers MMIO PTEs. Move it off-MMIO."
            )
        elif base >= RAM_BASE:
            errors.append(
                f"{name}: base 0x{base:08X} is at/above RAM 0x{RAM_BASE:08X} ({rel}) — "
                f"collides with the kernel. Move it below RAM."
            )
        elif base < MIN_BASE:
            errors.append(
                f"{name}: base 0x{base:08X} is below 0x{MIN_BASE:08X} ({rel}) — "
                f"inside the null-trap region. Raise it."
            )

    # Duplicate-base collision (two cells loading at the same fixed VA).
    seen: dict[int, str] = {}
    for name, base, rel in cells:
        if base in seen:
            errors.append(
                f"{name}: base 0x{base:08X} ({rel}) duplicates {seen[base]} — "
                f"two cells cannot share a load VA in the SAS."
            )
        else:
            seen[base] = name

    print("Cell VA layout:")
    for name, base, rel in sorted(cells, key=lambda c: c[1]):
        print(f"  0x{base:08X}  {name:<14} {rel}")
    print("MMIO windows (must stay in sync with paging.rs):")
    for nm, lo, hi in MMIO_WINDOWS:
        print(f"  0x{lo:08X}-0x{hi:08X}  {nm}")

    if errors:
        print("\nVA-LAYOUT VIOLATIONS:")
        for e in errors:
            print(f"  ✗ {e}")
        return 1

    print(f"\nOK — {len(cells)} cells, all off-MMIO, below RAM, unique bases.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
