#!/usr/bin/env bash
# CLI round-trip test for Ed25519 cell signing.
#
# Tests: sign → verify (pass) → tamper PT_LOAD byte → verify (fail, exit 3)
#
# Requires:
#   - python3 with `cryptography` package installed
#   - riscv-none-elf-objcopy (or $OBJCOPY override) in PATH
#   - A pre-built app-shell or app-init ELF in the release dir
#
# Usage:
#   bash scripts/test-cell-signing.sh
#   OBJCOPY=riscv-none-elf-objcopy bash scripts/test-cell-signing.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SIGN="$SCRIPT_DIR/sign-cell.py"
OBJCOPY="${OBJCOPY:-riscv-none-elf-objcopy}"

REL_DIR="$REPO_ROOT/target/riscv64gc-unknown-none-elf/release"
# Prefer app-shell; fall back to app-init; skip if neither is built.
ELF=""
for candidate in "$REL_DIR/app-shell" "$REL_DIR/app-init"; do
    if [ -f "$candidate" ]; then
        ELF="$candidate"
        break
    fi
done

if [ -z "$ELF" ]; then
    echo "SKIP: no release cell found in $REL_DIR"
    echo "      Run: cargo build --release -p app-shell"
    exit 0
fi

TMP_DIR="$REPO_ROOT/target"
TMP_SIGNED="$TMP_DIR/test-signing-signed.elf"
TMP_TAMPERED="$TMP_DIR/test-signing-tampered.elf"

cleanup() { rm -f "$TMP_SIGNED" "$TMP_TAMPERED"; }
trap cleanup EXIT

echo "--- test-cell-signing ---"
echo "  ELF:     $ELF"
echo "  OBJCOPY: $OBJCOPY"
echo

# ── Step 1: Sign ──────────────────────────────────────────────────────────────
OBJCOPY="$OBJCOPY" python3 "$SIGN" --in "$ELF" --out "$TMP_SIGNED"
echo "PASS: sign"

# ── Step 2: Verify signed output — must succeed (exit 0) ─────────────────────
OBJCOPY="$OBJCOPY" python3 "$SIGN" --verify --in "$TMP_SIGNED"
echo "PASS: verify (valid cell)"

# ── Step 3: Tamper a byte inside a PT_LOAD segment, then verify — must fail ──
cp "$TMP_SIGNED" "$TMP_TAMPERED"

# Use Python to locate the first PT_LOAD segment and flip a byte in its middle.
# This is robust: it reads actual phdr offsets instead of assuming a fixed offset.
python3 - "$TMP_TAMPERED" <<'PYEOF'
import sys
import struct

with open(sys.argv[1], 'r+b') as f:
    data = bytearray(f.read())

magic = data[0:4]
if magic != b'\x7fELF':
    print("ERROR: not an ELF file", file=sys.stderr)
    sys.exit(1)

ei_class = data[4]
if ei_class != 2:
    print("ERROR: only ELF64 supported (ei_class={})".format(ei_class), file=sys.stderr)
    sys.exit(1)

# ELF64 header field offsets
e_phoff     = struct.unpack_from('<Q', data, 32)[0]
e_phentsize = struct.unpack_from('<H', data, 54)[0]
e_phnum     = struct.unpack_from('<H', data, 56)[0]

PT_LOAD = 1
tamper_offset = None
for i in range(e_phnum):
    base = e_phoff + i * e_phentsize
    p_type   = struct.unpack_from('<I', data, base)[0]
    p_offset = struct.unpack_from('<Q', data, base + 8)[0]
    p_filesz = struct.unpack_from('<Q', data, base + 32)[0]
    if p_type == PT_LOAD and p_filesz >= 16:
        # Flip a byte near the middle of the segment.
        tamper_offset = p_offset + p_filesz // 2
        break

if tamper_offset is None:
    print("ERROR: no PT_LOAD segment with >= 16 bytes found", file=sys.stderr)
    sys.exit(1)

print("  tamper at file offset 0x{:x} (PT_LOAD mid-point)".format(tamper_offset))
data[tamper_offset] ^= 0xFF

with open(sys.argv[1], 'wb') as f:
    f.write(data)
PYEOF

# verify must now fail (exit 3 per sign-cell.py convention)
if OBJCOPY="$OBJCOPY" python3 "$SIGN" --verify --in "$TMP_TAMPERED" 2>/dev/null; then
    echo "FAIL: tampered cell should NOT verify — signature gate is broken"
    exit 1
fi
echo "PASS: verify (tampered cell correctly rejected)"

echo
echo "--- test-cell-signing: ALL PASS ---"
