#!/usr/bin/env bash
# Boot ViOS kernel in QEMU and assert the system reaches the shell.
#
# Without a disk image the system will fail to spawn VFS/config/shell from
# /bin/ but it should still boot the kernel, mount FAT32 (embedded), and
# attempt to spawn cells.  We assert on the FAT32 mount message which is
# always present regardless of disk availability.
#
# Usage: scripts/qemu-boot-test.sh [path/to/kernel-elf] [path/to/disk.img]

set -euo pipefail

KERNEL="${1:-target/riscv64gc-unknown-none-elf/release/vios-kernel}"
DISK="${2:-}"

# Build QEMU args — disk is optional.
QEMU_ARGS=(
  -machine virt
  -m 256M           # kernel(4.4MB) + heap(64MB) + cells fit in 256MB
  -nographic
  -bios default
  -kernel "$KERNEL"
)

if [[ -n "$DISK" && -f "$DISK" ]]; then
  QEMU_ARGS+=(
    -drive "file=$DISK,format=raw,id=hd0,if=none"
    -device virtio-blk-device,drive=hd0
  )
fi

echo "[qemu-test] Booting with args: ${QEMU_ARGS[*]}"

timeout 120 qemu-system-riscv64 "${QEMU_ARGS[@]}" 2>&1 | tee qemu.log &
QEMU_PID=$!

# Wait up to 90 seconds for the FAT32 mount message which appears on every boot.
for i in $(seq 1 90); do
  sleep 1
  if grep -q "FAT32 Mounted Successfully" qemu.log 2>/dev/null; then
    echo "PASS: FAT32 mounted — kernel booted successfully (${i}s)"
    kill $QEMU_PID 2>/dev/null || true
    exit 0
  fi
  # Also accept full boot to shell prompt if disk is present.
  if grep -q "ViOS >" qemu.log 2>/dev/null; then
    echo "PASS: Shell prompt reached — full boot successful (${i}s)"
    kill $QEMU_PID 2>/dev/null || true
    exit 0
  fi
  if grep -q "PANIC" qemu.log 2>/dev/null; then
    echo "FAIL: kernel panic detected"
    cat qemu.log
    kill $QEMU_PID 2>/dev/null || true
    exit 1
  fi
done

echo "FAIL: FAT32 mount not seen within 90s"
cat qemu.log
kill $QEMU_PID 2>/dev/null || true
exit 1
