#!/usr/bin/env bash
# Boot ViOS on QEMU x86_64 q35 machine over COM1 serial output.
set -euo pipefail

KERNEL="${1:-target/x86_64-unknown-none/release/vios-kernel}"

qemu-system-x86_64 \
  -machine q35 \
  -cpu qemu64 \
  -m 256M \
  -nographic \
  -serial mon:stdio \
  -kernel "$KERNEL"
