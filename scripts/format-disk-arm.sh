#!/usr/bin/env bash
# Create disk_arm_virt.img for AArch64 QEMU boot testing.
#
# Uses tools/mkfat32.py (pure Python, no mtools required) — same tool as the
# Windows format-disk-arm.ps1 companion.
#
# Usage: bash scripts/format-disk-arm.sh [output.img]
#   output.img  default: disk_arm_virt.img

set -euo pipefail

OUT="${1:-disk_arm_virt.img}"
TARGET="aarch64-unknown-none-softfloat"
PROFILE="release"
BIN_DIR="target/$TARGET/$PROFILE"

echo "[format-disk-arm] Collecting cell binaries from $BIN_DIR..."

declare -A CELLS=(
    [app-init]=init
    [app-shell]=shell
    [service-vfs]=vfs
    [service-config]=config
    [service-net]=net
    [service-input]=input
    [service-compositor]=compositor
)

MKFAT_ARGS=()
for src_name in "${!CELLS[@]}"; do
    dst_name="${CELLS[$src_name]}"
    src="$BIN_DIR/$src_name"
    if [[ -f "$src" ]]; then
        echo "  /bin/$dst_name <- $src"
        MKFAT_ARGS+=("$src" "/bin/$dst_name")
    else
        echo "  WARNING: $src not found, skipping /bin/$dst_name"
    fi
done

# Include /etc/hostname
HOSTNAME_TMP=$(mktemp)
echo "ViCell-ARM" > "$HOSTNAME_TMP"
MKFAT_ARGS+=("$HOSTNAME_TMP" "/etc/hostname")

echo "[format-disk-arm] Creating $OUT with tools/mkfat32.py..."
python3 tools/mkfat32.py "$OUT" "${MKFAT_ARGS[@]}"

rm -f "$HOSTNAME_TMP"
echo "[format-disk-arm] Done: $OUT"
