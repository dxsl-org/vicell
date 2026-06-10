#!/usr/bin/env bash
# Downloads the Limine RISC-V UEFI bootloader binary for local/CI use.
#
# NOTE: Limine v9+ no longer ships a standalone S-mode ELF binary.
# perf.yml now boots the kernel directly (no Limine required for bench).
# This script extracts BOOTRISCV64.EFI from the limine-binary tarball for
# any future UEFI-boot workflows.
#
# Usage: ./scripts/download-limine.sh [output-path]
# Default output: tools/limine-riscv64

set -euo pipefail

LIMINE_TAG="v12.3.2"
LIMINE_URL="https://github.com/Limine-Bootloader/Limine/releases/download/${LIMINE_TAG}/limine-binary.tar.gz"
DEST="${1:-tools/limine-riscv64}"

mkdir -p "$(dirname "$DEST")"

if [[ -f "$DEST" ]]; then
  echo "[limine] Already present: $DEST"
  exit 0
fi

echo "[limine] Downloading Limine ${LIMINE_TAG} binary tarball..."
TMPTAR=$(mktemp /tmp/limine-binary.XXXXXX.tar.gz)

if command -v curl &>/dev/null; then
  curl -fsSL -o "$TMPTAR" "$LIMINE_URL"
elif command -v wget &>/dev/null; then
  wget -q -O "$TMPTAR" "$LIMINE_URL"
else
  echo "[limine] ERROR: neither curl nor wget found" >&2
  exit 1
fi

tar -xOf "$TMPTAR" "limine-binary/BOOTRISCV64.EFI" > "$DEST"
chmod +x "$DEST"
rm -f "$TMPTAR"

echo "[limine] Saved BOOTRISCV64.EFI to $DEST ($(du -sh "$DEST" | cut -f1))"
echo "[limine] NOTE: this is a UEFI EFI binary, not an S-mode ELF."
echo "[limine]       perf.yml uses direct kernel boot — this is for UEFI workflows only."
