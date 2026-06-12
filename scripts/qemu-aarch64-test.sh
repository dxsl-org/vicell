#!/usr/bin/env bash
# Boot the ViCell AArch64 kernel in QEMU and assert the system reaches the shell
# prompt ("ViCell >").
#
# Mirrors scripts/qemu-boot-test.sh for the ARM64 virt machine.
#
# Usage: scripts/qemu-aarch64-test.sh [kernel-elf] [disk.img]
#   kernel-elf  default: target/aarch64-unknown-none-softfloat/release/vicell-kernel
#   disk.img    default: disk_arm_virt.img

set -euo pipefail

KERNEL="${1:-target/aarch64-unknown-none-softfloat/release/vicell-kernel}"
DISK="${2:-disk_arm_virt.img}"
BOOT_WINDOW="${BOOT_WINDOW:-55}"

if ! command -v qemu-system-aarch64 &>/dev/null; then
    echo "FAIL: qemu-system-aarch64 not found on PATH" >&2
    exit 1
fi

if [[ ! -f "$KERNEL" ]]; then
    echo "FAIL: kernel ELF not found: $KERNEL" >&2
    echo "  Build with: RUSTFLAGS='-C relocation-model=pic' cargo build --release --target aarch64-unknown-none-softfloat -p vicell-kernel" >&2
    exit 1
fi

if [[ ! -f "$DISK" ]]; then
    echo "FAIL: disk image not found: $DISK" >&2
    echo "  Build with: .\\format-disk-arm.ps1  (or tools/mkfat32.py)" >&2
    exit 1
fi

echo "[qemu-aarch64-test] Booting kernel=$KERNEL disk=$DISK (window=${BOOT_WINDOW}s)"

QEMU_ARGS=(
    -machine virt
    -cpu cortex-a57
    -m 256M
    -nographic
    -kernel "$KERNEL"
    -drive "if=none,file=$DISK,format=raw,id=hd0"
    -device virtio-blk-device,drive=hd0
    -netdev user,id=net0
    -device virtio-net-device,netdev=net0
    -no-reboot
    -serial stdio
)
# Add VirtIO RNG only when /dev/random is available (Linux CI).
# On Windows QEMU 10, rng-random is invalid; skip it — boot tests don't need RNG.
if [[ -c /dev/random ]]; then
    QEMU_ARGS+=(-object rng-random,id=rng0 -device virtio-rng-device,rng=rng0)
fi

timeout "$BOOT_WINDOW" qemu-system-aarch64 "${QEMU_ARGS[@]}" \
    < /dev/null > qemu-aarch64.raw.log 2>&1 || true

# Strip NULs and ANSI escape sequences so patterns match cleanly.
tr -d '\000' < qemu-aarch64.raw.log | sed 's/\x1b\[[0-9;]*m//g' > qemu-aarch64.log

if grep -qia "KERNEL PANIC\|\[fault\] Cell" qemu-aarch64.log; then
    echo "FAIL: kernel panic / cell fault detected during aarch64 boot" >&2
    grep -ai "fault\|PANIC" qemu-aarch64.log | head
    exit 1
fi

if grep -q "ViCell >" qemu-aarch64.log; then
    echo "PASS: aarch64 shell prompt reached — full boot successful"
    exit 0
fi

echo "FAIL: 'ViCell >' prompt not seen within ${BOOT_WINDOW}s" >&2
tail -40 qemu-aarch64.log
exit 1
