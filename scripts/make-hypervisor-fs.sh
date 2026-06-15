#!/usr/bin/env bash
# Build a hypervisor-specific kernel_fs.img that embeds Alpine Linux artifacts.
#
# The ViCell hypervisor cell reads /vmlinuz and /initrd.gz from VIFS1 (the kernel's
# embedded FAT image).  This script:
#   1. Fetches Alpine aarch64 netboot artifacts (via fetch-alpine-artifacts.sh).
#   2. Builds the aarch64 cells (including service-hypervisor).
#   3. Creates kernel/src/embedded-hv/kernel_fs.img with:
#        - Regular boot cells (init, shell, vfs, config) from existing embedded img
#        - /vmlinuz   (Alpine vmlinuz-virt)
#        - /initrd.gz (Alpine initramfs-virt, renamed from initramfs-virt)
#   4. Prints the EMBEDDED_OVERRIDE instruction for the kernel build.
#
# After running this script, rebuild the kernel with:
#   RUSTFLAGS="-C relocation-model=pic" \
#   EMBEDDED_OVERRIDE="kernel/src/embedded-hv" \
#   cargo build --release -p vicell-kernel --target aarch64-unknown-none-softfloat
#
# Usage: bash scripts/make-hypervisor-fs.sh [--skip-fetch]

set -euo pipefail

SKIP_FETCH="${1:-}"
TARGET="aarch64-unknown-none-softfloat"
PROFILE="release"
BIN_DIR="target/$TARGET/$PROFILE"
ALPINE_CACHE=".alpine-cache"
EMBEDDED_SRC="kernel/src/embedded-aarch64"
EMBEDDED_HV="kernel/src/embedded-hv"
STAGING=$(mktemp -d)

cleanup() { rm -rf "$STAGING"; }
trap cleanup EXIT

# ── Step 1: Fetch Alpine artifacts ──────────────────────────────────────────
if [[ "$SKIP_FETCH" != "--skip-fetch" ]]; then
    echo "[make-hv-fs] Fetching Alpine artifacts..."
    bash scripts/fetch-alpine-artifacts.sh "$ALPINE_CACHE"
fi

if [[ ! -f "$ALPINE_CACHE/vmlinuz-virt" || ! -f "$ALPINE_CACHE/initramfs-virt" ]]; then
    echo "ERROR: Alpine artifacts missing from $ALPINE_CACHE/" >&2
    echo "  Run: bash scripts/fetch-alpine-artifacts.sh" >&2
    exit 1
fi

# ── Step 2: Build aarch64 cells (including service-hypervisor) ──────────────
echo "[make-hv-fs] Building aarch64 cells (service-hypervisor + core cells)..."
RUSTFLAGS="-D warnings" cargo build --release \
    --target "$TARGET" \
    -Z build-std=core,alloc \
    -p app-init -p app-shell -p service-vfs -p service-config \
    -p service-net -p service-hypervisor

# ── Step 3: Build hypervisor kernel_fs.img ──────────────────────────────────
mkdir -p "$EMBEDDED_HV"

echo "[make-hv-fs] Building hypervisor kernel_fs.img..."

# Collect args for mkfat32.py: (src, dst) pairs
MKFAT_ARGS=()

# Copy embedded cells from existing aarch64 embedded dir or fresh build.
for cell in init shell vfs config; do
    src="$BIN_DIR/app-$cell"
    [[ ! -f "$src" ]] && src="$BIN_DIR/service-$cell"
    [[ ! -f "$src" ]] && src="$EMBEDDED_SRC/$cell"
    if [[ -f "$src" ]]; then
        echo "  /$cell <- $src"
        MKFAT_ARGS+=("$src" "/$cell")
    else
        echo "  WARNING: $cell not found — skipping"
    fi
done

# Hypervisor cell itself (loaded from FAT32 disk by init, not from kernel_fs).
# kernel_fs only needs the base cells + Alpine images.

# Alpine kernel and initrd (FAT16 paths are uppercased by the kernel; store as-is).
VMLINUZ="$ALPINE_CACHE/vmlinuz-virt"
INITRD="$ALPINE_CACHE/initramfs-virt"

echo "  /vmlinuz   <- $VMLINUZ ($(du -sh "$VMLINUZ" | cut -f1))"
MKFAT_ARGS+=("$VMLINUZ" "/vmlinuz")

echo "  /initrd.gz <- $INITRD ($(du -sh "$INITRD" | cut -f1))"
MKFAT_ARGS+=("$INITRD" "/initrd.gz")

python3 tools/mkfat32.py "$EMBEDDED_HV/kernel_fs.img" "${MKFAT_ARGS[@]}"

echo ""
echo "[make-hv-fs] kernel_fs.img created at $EMBEDDED_HV/kernel_fs.img"
ls -lh "$EMBEDDED_HV/kernel_fs.img"
echo ""
echo "Next — build the hypervisor kernel:"
echo "  export RUSTFLAGS='-C relocation-model=pic'"
echo "  export EMBEDDED_OVERRIDE='$EMBEDDED_HV'"
echo "  cargo build --release -p vicell-kernel --target $TARGET -Z build-std=core,alloc"
echo "  unset RUSTFLAGS EMBEDDED_OVERRIDE"
echo ""
echo "Then build the hypervisor disk:"
echo "  bash scripts/format-disk-hv-arm.sh"
