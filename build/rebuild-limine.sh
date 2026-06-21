#!/bin/bash
set -e
TOOLS=/tmp/tools-local
NASM=/tmp/nasm-local
export PATH="$TOOLS/usr/bin:$NASM/usr/bin:$PATH"

cd /mnt/d/ViCell/limine/limine-8.7.0

echo "=== Checking tools ==="
which nasm && nasm --version | head -1
which mcopy && mcopy --version | head -1
which mformat && mformat --version 2>&1 | head -1

echo "=== Reconfiguring with mcopy ==="
./configure \
    --enable-bios \
    --enable-bios-cd \
    --enable-uefi-x86-64 \
    --enable-uefi-cd \
    2>&1 | grep -E '(checking for mtools|mcopy|configure:)'

echo "=== Building limine-uefi-cd (phony target) ==="
make -j$(nproc) limine-uefi-cd 2>&1

echo "=== Verifying ==="
ls -lh bin/limine-uefi-cd.bin && echo "BUILD_SUCCESS" || echo "BUILD_FAILED"
