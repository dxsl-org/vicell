#!/usr/bin/env bash
# Boot ViOS on QEMU aarch64 virt machine.
set -euo pipefail

KERNEL="${1:-target/aarch64-unknown-none/release/vios-kernel}"

qemu-system-aarch64 \
  -machine virt,gic-version=2 \
  -cpu cortex-a72 \
  -m 256M \
  -nographic \
  -kernel "$KERNEL"
