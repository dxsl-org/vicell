#!/usr/bin/env bash
# build-mlibc.sh — Build mlibc libc.a for ViCell (riscv64 + aarch64) in WSL2.
#
# Flow (mirrors scripts/setup-mlibc.ps1 for Windows):
#   1. Clone managarm/mlibc to third_party/mlibc-src/ (skip if already cloned)
#   2. Copy ViCell sysdeps/vicell into the clone
#   3. Patch mlibc's meson.build to add the 'vicell' host_machine.system() branch
#   4. Build riscv64 + aarch64 with meson/ninja
#   5. Copy libc.a outputs to third_party/mlibc/build[-aarch64]/libc.a
#
# Prerequisites (one-time setup in WSL2):
#   sudo apt update && sudo apt install -y meson ninja-build \
#       gcc-aarch64-linux-gnu g++-aarch64-linux-gnu
#
# riscv64 toolchain: xpack riscv-none-elf-gcc at /mnt/c/RISCV (Windows path via WSL2).
# aarch64: uses system aarch64-linux-gnu-gcc from apt.
#
# Usage:
#   cd /mnt/d/ViCell    # WSL2 path to repo root
#   bash scripts/build-mlibc.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MLIBC_SRC_DIR="$REPO_ROOT/third_party/mlibc-src"
OUR_SYSDEPS="$REPO_ROOT/third_party/mlibc/sysdeps/vicell"
BUILD_OUT_RV="$REPO_ROOT/third_party/mlibc/build"
BUILD_OUT_A64="$REPO_ROOT/third_party/mlibc/build-aarch64"
SCRIPTS="$REPO_ROOT/scripts"
MLIBC_REPO="https://github.com/managarm/mlibc.git"

# ─── Step 1: Clone / update mlibc source ─────────────────────────────────────
echo "=== Step 1: mlibc source ==="
if [[ -d "$MLIBC_SRC_DIR/.git" ]]; then
    echo "  Already cloned — pulling latest..."
    git -C "$MLIBC_SRC_DIR" pull --ff-only
else
    echo "  Cloning $MLIBC_REPO ..."
    mkdir -p "$MLIBC_SRC_DIR"
    git clone --depth 1 "$MLIBC_REPO" "$MLIBC_SRC_DIR"
fi
echo "  mlibc commit: $(git -C "$MLIBC_SRC_DIR" rev-parse HEAD)"

# ─── Step 2: Install ViCell sysdeps ──────────────────────────────────────────
echo "=== Step 2: Installing ViCell sysdeps ==="
DEST_SYSDEPS="$MLIBC_SRC_DIR/sysdeps/vicell"
mkdir -p "$DEST_SYSDEPS"
cp -r "$OUR_SYSDEPS"/. "$DEST_SYSDEPS/"
echo "  Copied sysdeps/vicell → $DEST_SYSDEPS"

# ─── Step 3: Patch mlibc's meson.build ───────────────────────────────────────
echo "=== Step 3: Patching meson.build ==="
MESON_BUILD="$MLIBC_SRC_DIR/meson.build"
if ! grep -q "host_machine.system() == 'vicell'" "$MESON_BUILD"; then
    # Insert the vicell branch before the final else/error block
    PATCH=$'\nelif host_machine.system() == \'vicell\'\n    subdir(\'sysdeps/vicell\')\n'
    if python3 - "$MESON_BUILD" "$PATCH" <<'PYEOF'
import sys, re

path = sys.argv[1]
patch = sys.argv[2]

with open(path) as f:
    content = f.read()

# Insert before the closing else..error() block
new_content = re.sub(
    r"(elif host_machine\.system\(\) == '[^']+'\s*\n\s*subdir\('[^']+'\)\s*\n)(else\s*\n\s*error\()",
    r"\g<1>" + patch + r"\n\g<2>",
    content,
    count=1,
)

if new_content == content:
    # Fallback: insert before the last else block
    new_content = re.sub(
        r"(else\s*\n\s*error\('Unknown OS)",
        patch + r"\n\g<1>",
        content,
        count=1,
    )

if "host_machine.system() == 'vicell'" in new_content:
    with open(path, 'w') as f:
        f.write(new_content)
    sys.exit(0)
else:
    sys.exit(1)
PYEOF
    then
        echo "  meson.build patched successfully"
    else
        echo "ERROR: failed to patch meson.build — insert vicell branch manually" >&2
        exit 1
    fi
else
    echo "  meson.build already has vicell branch — skipping"
fi

COMMON_OPTS=(
    -Ddefault_library=static
    -Dposix_option=disabled
    -Dlinux_option=disabled
    -Dheaders_only=false
)

# ─── Step 4a: riscv64 ────────────────────────────────────────────────────────
echo "=== Step 4a: meson + ninja (riscv64) ==="
BUILD_RV="$MLIBC_SRC_DIR/build"
meson setup "$BUILD_RV" "$MLIBC_SRC_DIR" \
    --cross-file="$SCRIPTS/mlibc-riscv64.cross" \
    "${COMMON_OPTS[@]}" \
    --wipe 2>/dev/null || \
meson setup "$BUILD_RV" "$MLIBC_SRC_DIR" \
    --cross-file="$SCRIPTS/mlibc-riscv64.cross" \
    "${COMMON_OPTS[@]}"

ninja -C "$BUILD_RV"
mkdir -p "$BUILD_OUT_RV"
cp "$BUILD_RV/libc.a" "$BUILD_OUT_RV/libc.a"
echo "  riscv64 libc.a: $(du -sh "$BUILD_OUT_RV/libc.a" | cut -f1)"

# ─── Step 4b: aarch64 ────────────────────────────────────────────────────────
echo "=== Step 4b: meson + ninja (aarch64) ==="
BUILD_A64="$MLIBC_SRC_DIR/build-aarch64"
meson setup "$BUILD_A64" "$MLIBC_SRC_DIR" \
    --cross-file="$SCRIPTS/mlibc-aarch64.cross" \
    "${COMMON_OPTS[@]}" \
    --wipe 2>/dev/null || \
meson setup "$BUILD_A64" "$MLIBC_SRC_DIR" \
    --cross-file="$SCRIPTS/mlibc-aarch64.cross" \
    "${COMMON_OPTS[@]}"

ninja -C "$BUILD_A64"
mkdir -p "$BUILD_OUT_A64"
cp "$BUILD_A64/libc.a" "$BUILD_OUT_A64/libc.a"
echo "  aarch64 libc.a: $(du -sh "$BUILD_OUT_A64/libc.a" | cut -f1)"

echo ""
echo "✅ mlibc build complete!"
echo "   third_party/mlibc/build/libc.a          (riscv64)"
echo "   third_party/mlibc/build-aarch64/libc.a  (aarch64)"
echo "   Run 'cargo check' — mlibc-shim warning should be gone."
