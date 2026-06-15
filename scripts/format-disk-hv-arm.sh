#!/usr/bin/env bash
# Create disk_hv_arm.img — the FAT32 user disk for the ViCell hypervisor boot.
#
# Contains the hypervisor cell + core cells.  The kernel_fs.img (with Alpine
# artifacts) is embedded in the kernel binary via EMBEDDED_OVERRIDE; this disk
# provides the FAT32 filesystem that init mounts and spawns /bin/hypervisor from.
#
# Usage: bash scripts/format-disk-hv-arm.sh [output.img]

set -euo pipefail

OUT="${1:-disk_hv_arm.img}"
TARGET="aarch64-unknown-none-softfloat"
BIN_DIR="target/$TARGET/release"

echo "[format-disk-hv] Collecting aarch64 cell binaries from $BIN_DIR..."

declare -A CELLS=(
    [app-init]=init
    [app-shell]=shell
    [service-vfs]=vfs
    [service-config]=config
    [service-net]=net
    [service-input]=input
    [service-compositor]=compositor
    [service-hypervisor]=hypervisor
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

HOSTNAME_TMP=$(mktemp)
echo "ViCell-HV" > "$HOSTNAME_TMP"
MKFAT_ARGS+=("$HOSTNAME_TMP" "/etc/hostname")

echo "[format-disk-hv] Creating $OUT with tools/mkfat32.py..."
python3 tools/mkfat32.py "$OUT" "${MKFAT_ARGS[@]}"

rm -f "$HOSTNAME_TMP"
echo "[format-disk-hv] Done: $OUT"
