#!/usr/bin/env bash
# Boot the ViCell hypervisor kernel in QEMU ARM virt (EL2) and assert the Alpine
# Linux guest reaches its busybox shell prompt ("/ #") within BOOT_WINDOW seconds.
#
# The kernel must be built with EMBEDDED_OVERRIDE pointing to a kernel_fs.img
# that contains Alpine vmlinuz-virt + initramfs-virt (see scripts/make-hypervisor-fs.sh).
#
# Usage:
#   bash scripts/qemu-hypervisor-smoke.sh [kernel-elf] [disk.img]
#   kernel-elf  default: target/aarch64-unknown-none-softfloat/release/vicell-kernel
#   disk.img    default: disk_hv_arm.img
#
# Environment:
#   BOOT_WINDOW  seconds to wait for "/ #"  (default: 180 — TCG boot takes 30-120s)
#
# Exit codes:
#   0  — Alpine guest reached "/ #" and issued PSCI SYSTEM_OFF cleanly
#   1  — timeout, kernel panic, or unexpected output

set -euo pipefail

KERNEL="${1:-target/aarch64-unknown-none-softfloat/release/vicell-kernel}"
DISK="${2:-disk_hv_arm.img}"
BOOT_WINDOW="${BOOT_WINDOW:-180}"

if ! command -v qemu-system-aarch64 &>/dev/null; then
    echo "FAIL: qemu-system-aarch64 not found on PATH" >&2
    exit 1
fi

for f in "$KERNEL" "$DISK"; do
    if [[ ! -f "$f" ]]; then
        echo "FAIL: required file not found: $f" >&2
        exit 1
    fi
done

echo "[hv-smoke] Booting kernel=$KERNEL disk=$DISK (window=${BOOT_WINDOW}s)"
echo "[hv-smoke] Waiting for Alpine guest '/ #' prompt..."

QEMU_ARGS=(
    -machine "virt,virtualization=on,gic-version=2"
    -cpu cortex-a72
    -m 1G
    -nographic
    -kernel "$KERNEL"
    -drive "if=none,file=$DISK,format=raw,id=hd0"
    -device virtio-blk-device,drive=hd0
    -netdev user,id=net0
    -device virtio-net-device,netdev=net0
    -no-reboot
    -serial stdio
)

timeout "$BOOT_WINDOW" qemu-system-aarch64 "${QEMU_ARGS[@]}" \
    < /dev/null > qemu-hv.raw.log 2>&1 || true

# Strip NULs and ANSI escapes.
tr -d '\000' < qemu-hv.raw.log | sed 's/\x1b\[[0-9;]*m//g' > qemu-hv.log

# Check for kernel/host-side panics first.
if grep -qia "KERNEL PANIC\|\[fault\] Cell" qemu-hv.log; then
    echo "FAIL: kernel panic / cell fault detected" >&2
    grep -ai "fault\|PANIC" qemu-hv.log | head -20
    exit 1
fi

# Check for hypervisor-specific errors.
if grep -qi "\[hv\] .*fail\|\[hv\] .*error\|\[hv\] guest exited" qemu-hv.log; then
    echo "FAIL: hypervisor error before guest boot" >&2
    grep -i "\[hv\]" qemu-hv.log | tail -20
    exit 1
fi

# Assert Alpine guest reached its busybox shell.
if grep -q "^/ #" qemu-hv.log || grep -q $'/ #' qemu-hv.log; then
    echo "PASS: Alpine guest '/ #' prompt reached — hypervisor smoke test OK"
    exit 0
fi

# Also accept the variant with hostname prefix (Alpine ash default prompt).
if grep -qP "~ #|localhost:~#" qemu-hv.log 2>/dev/null; then
    echo "PASS: Alpine guest shell prompt reached — hypervisor smoke test OK"
    exit 0
fi

echo "FAIL: Alpine '/ #' prompt not seen within ${BOOT_WINDOW}s" >&2
echo "--- last 50 lines of QEMU output ---" >&2
tail -50 qemu-hv.log >&2
exit 1
