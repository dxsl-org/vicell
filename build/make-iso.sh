#!/bin/bash
set -e
TOOLS=/tmp/tools-local
export LD_LIBRARY_PATH="$TOOLS/usr/lib/x86_64-linux-gnu:$TOOLS/usr/lib:$TOOLS/lib/x86_64-linux-gnu"
XORRISO="$TOOLS/usr/bin/xorriso"

ISO_ROOT=/mnt/d/ViCell/build/x86-iso-root
ISO_OUT=/mnt/d/ViCell/build/vicell-x86.iso
LIMINE=/mnt/d/ViCell/limine/limine-8.7.0/bin
KERNEL=/mnt/d/ViCell/target/x86_64-unknown-none/release/vicell-kernel

mkdir -p "$ISO_ROOT/EFI/BOOT"
mkdir -p "$ISO_ROOT/boot/limine"

cp "$KERNEL" "$ISO_ROOT/boot/kernel.elf"
# Limine v8 uses limine.conf (new format) — limine.cfg triggers 20s compat warning
cp /mnt/d/ViCell/scripts/x86/limine.conf "$ISO_ROOT/boot/limine.conf"
cp "$LIMINE/limine-bios-cd.bin" "$ISO_ROOT/boot/limine/"
cp "$LIMINE/limine-bios.sys" "$ISO_ROOT/boot/limine/"
cp "$LIMINE/BOOTX64.EFI" "$ISO_ROOT/EFI/BOOT/"
# limine-uefi-cd.bin is a pre-formatted FAT12 image containing EFI/BOOT/*.EFI
# It must be used as the EFI El Torito boot image (not BOOTX64.EFI directly).
cp "$LIMINE/limine-uefi-cd.bin" "$ISO_ROOT/boot/limine/"

echo "Building ISO..."
$XORRISO -as mkisofs \
  -b boot/limine/limine-bios-cd.bin \
  -no-emul-boot -boot-load-size 4 -boot-info-table \
  --efi-boot boot/limine/limine-uefi-cd.bin -efi-boot-part --efi-boot-image \
  -o "$ISO_OUT" "$ISO_ROOT" 2>&1

echo "ISO_SIZE=$(stat -c %s "$ISO_OUT") bytes"
echo "ISO_READY"
