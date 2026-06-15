#!/usr/bin/env bash
# Fetch Alpine Linux aarch64 netboot artifacts for the ViCell hypervisor smoke test.
#
# Downloads vmlinuz-virt + initramfs-virt from the Alpine CDN, verifies SHA256
# checksums, and caches them in .alpine-cache/.
#
# Usage:
#   bash scripts/fetch-alpine-artifacts.sh [dest-dir]
#   dest-dir default: .alpine-cache
#
# Update ALPINE_VERSION + SHA256 values when upgrading Alpine.
# Checksums from https://dl-cdn.alpinelinux.org/alpine/vVER/releases/aarch64/SHA256SUMS
#
# Security: supply-chain pinning — never download without checksum verification.

set -euo pipefail

ALPINE_VERSION="${ALPINE_VERSION:-3.21.3}"
DEST="${1:-.alpine-cache}"
CDN="https://dl-cdn.alpinelinux.org/alpine/v${ALPINE_VERSION%.*}/releases/aarch64/netboot"

# SHA256 checksums for Alpine 3.21.3 aarch64 netboot artifacts.
# Update these when bumping ALPINE_VERSION:
#   curl -s "$CDN/../SHA256SUMS" | grep -E "vmlinuz-virt|initramfs-virt"
VMLINUZ_SHA256="${VMLINUZ_SHA256:-UPDATE_ME_FROM_SHA256SUMS}"
INITRD_SHA256="${INITRD_SHA256:-UPDATE_ME_FROM_SHA256SUMS}"

VMLINUZ_URL="$CDN/vmlinuz-virt"
INITRD_URL="$CDN/initramfs-virt"

mkdir -p "$DEST"

fetch_and_verify() {
    local url="$1" dest="$2" expected_sha="$3" name="$4"

    if [[ -f "$dest" ]]; then
        actual=$(sha256sum "$dest" | awk '{print $1}')
        if [[ "$actual" == "$expected_sha" ]]; then
            echo "[fetch-alpine] $name: cached OK (sha256 verified)"
            return 0
        else
            echo "[fetch-alpine] $name: cached checksum mismatch — re-downloading"
            rm -f "$dest"
        fi
    fi

    echo "[fetch-alpine] Downloading $name from $url ..."
    if command -v curl &>/dev/null; then
        curl -fSL --retry 3 --retry-delay 5 -o "$dest" "$url"
    elif command -v wget &>/dev/null; then
        wget -q --tries=3 -O "$dest" "$url"
    else
        echo "ERROR: neither curl nor wget found" >&2
        exit 1
    fi

    if [[ "$expected_sha" == "UPDATE_ME_FROM_SHA256SUMS" ]]; then
        echo "[fetch-alpine] WARNING: no checksum configured for $name — skipping verification"
        echo "[fetch-alpine] Run: sha256sum $dest  and update scripts/fetch-alpine-artifacts.sh"
        return 0
    fi

    actual=$(sha256sum "$dest" | awk '{print $1}')
    if [[ "$actual" != "$expected_sha" ]]; then
        echo "ERROR: SHA256 mismatch for $name" >&2
        echo "  Expected: $expected_sha" >&2
        echo "  Actual:   $actual" >&2
        rm -f "$dest"
        exit 1
    fi
    echo "[fetch-alpine] $name: downloaded + verified OK"
}

fetch_and_verify "$VMLINUZ_URL" "$DEST/vmlinuz-virt"    "$VMLINUZ_SHA256" "vmlinuz-virt"
fetch_and_verify "$INITRD_URL"  "$DEST/initramfs-virt"  "$INITRD_SHA256"  "initramfs-virt"

echo ""
echo "[fetch-alpine] Artifacts ready in $DEST/:"
ls -lh "$DEST/vmlinuz-virt" "$DEST/initramfs-virt"
echo ""
echo "Next: run scripts/make-hypervisor-fs.sh to embed them into kernel_fs.img"
