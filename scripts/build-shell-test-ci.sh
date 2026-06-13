#!/usr/bin/env bash
# Build the shell-test kernel for Phase E shell utility integration tests (Linux CI).
#
# The shell-test kernel is identical to the standard release kernel except that
# app-shell is compiled with --features shell_test, replacing the interactive
# REPL with the deterministic scenario harness in shell_test.rs.
#
# Produces: target/riscv64gc-unknown-none-elf/release/vicell-kernel-shell-test
#
# Prerequisites (the CI job installs these):
#   apt: gcc-riscv64-unknown-elf libclang-dev qemu-system-misc
#   rustup: nightly with rust-src component

set -euo pipefail

REL="target/riscv64gc-unknown-none-elf/release"
ST_DIR="kernel/src/embedded-shell-test"

# riscv64 cross-compiler required by littlefs2 C FFI.
export CC_riscv64gc_unknown_none_elf="riscv64-unknown-elf-gcc"
export CFLAGS_riscv64gc_unknown_none_elf="-march=rv64gc -mabi=lp64d -mcmodel=medany -ffreestanding -DLFS_NO_INTRINSICS"

echo "==> Building base cells (init, config)..."
cargo build --release \
    --target riscv64gc-unknown-none-elf \
    -Z build-std=core,alloc \
    -p app-init -p service-config

echo "==> Building VFS service..."
cargo build --release \
    --target riscv64gc-unknown-none-elf \
    -Z build-std=core,alloc \
    -p service-vfs

echo "==> Building shell (shell_test feature)..."
cargo build --release \
    --target riscv64gc-unknown-none-elf \
    -Z build-std=core,alloc \
    -p app-shell --features shell_test

echo "==> Verifying cell binaries..."
for bin in app-init app-shell service-vfs service-config; do
    if [[ ! -f "$REL/$bin" ]]; then
        echo "FAIL: missing required binary: $REL/$bin" >&2; exit 1
    fi
done

echo "==> Assembling kernel_fs.img (shell-test)..."
mkdir -p "$ST_DIR"
TMPDIR_KFS=$(mktemp -d)
printf 'ViCell-shell-test' > "$TMPDIR_KFS/hostname"

python3 tools/mkfat32.py \
    "$ST_DIR/kernel_fs.img" \
    "$REL/app-init"        /bin/init \
    "$REL/app-shell"       /bin/shell \
    "$REL/service-vfs"     /bin/vfs \
    "$REL/service-config"  /bin/config \
    "$TMPDIR_KFS/hostname" /etc/hostname

if [[ ! -f "$ST_DIR/kernel_fs.img" ]]; then
    echo "FAIL: mkfat32.py did not produce kernel_fs.img" >&2; exit 1
fi
echo "   kernel_fs.img: $(du -sh "$ST_DIR/kernel_fs.img" | cut -f1)"

# INIT_ELF (include_bytes!) is embedded separately from kernel_fs.img.
cp "$REL/app-init" "$ST_DIR/init"
echo "   init: $(du -sh "$ST_DIR/init" | cut -f1)"

echo "==> Building shell-test kernel (riscv64, PIC)..."
EMBEDDED_OVERRIDE="$ST_DIR" \
RUSTFLAGS="-D warnings -C relocation-model=pic" \
cargo build --release \
    --target riscv64gc-unknown-none-elf \
    -Z build-std=core,alloc \
    -p vicell-kernel

cp "$REL/vicell-kernel" "$REL/vicell-kernel-shell-test"
echo "==> Done: $REL/vicell-kernel-shell-test"
