# Run ViCell on QEMU ARM virt (aarch64) — peripheral driver integration tests.
#
# Prerequisites:
#   1. Install qemu-system-aarch64 (e.g., via winget or QEMU installer).
#   2. Build the aarch64 kernel:
#        cargo build --release -p vicell-kernel --target aarch64-unknown-none-softfloat
#   3. Build the aarch64 disk image with periph-test + robot-demo ELFs embedded.
#
# This script targets the QEMU ARM virt machine which provides:
#   PL011 UART  at 0x0900_0000  (serial console + periph-test UART loopback)
#   PL061 GPIO  at 0x0903_0000  (periph-test GPIO + robot-demo sensor/actuator)
#   GICv2       at 0x0800_0000  (interrupt controller — Phase ARM64)
#   Generic timer               (periodic timer — Phase ARM64)

$qemu = "qemu-system-aarch64"
if (-not (Get-Command $qemu -ErrorAction SilentlyContinue)) {
    if (Test-Path "C:\Program Files\qemu\qemu-system-aarch64.exe") {
        $qemu = "C:\Program Files\qemu\qemu-system-aarch64.exe"
    } else {
        Write-Host "qemu-system-aarch64 not found. Install QEMU and add it to PATH."
        exit 1
    }
}

$target  = "aarch64-unknown-none-softfloat"
$kernel  = "target/$target/release/vicell-kernel"
$disk    = "disk_arm_virt.img"

Write-Host "Building aarch64 release kernel..."
$env:RUSTFLAGS = "-C relocation-model=pic"
cargo build --release -p vicell-kernel --target $target
$env:RUSTFLAGS = $null

if (-not (Test-Path $kernel)) {
    Write-Host "aarch64 kernel build failed — check target installation:"
    Write-Host "  rustup target add aarch64-unknown-none-softfloat"
    exit 1
}

if (-not (Test-Path $disk)) {
    Write-Host "Disk image $disk not found."
    Write-Host "Build it with: python3 scripts/mkfat32_inplace.py --arm-virt"
    exit 1
}

Write-Host "Starting ViCell on QEMU ARM virt (aarch64)..."
Write-Host "Serial output: PL011 UART at 0x0900_0000"
Write-Host "GPIO:          PL061 at 0x0903_0000"
Write-Host "Press Ctrl-a x to quit QEMU."
Write-Host ""

& $qemu `
    -machine virt `
    -cpu cortex-a57 `
    -m 256M `
    -nographic `
    -kernel $kernel `
    -drive if=none,file=$disk,format=raw,id=hd0 `
    -device virtio-blk-device,drive=hd0 `
    -object rng-random,id=rng0 `
    -device virtio-rng-device,rng=rng0 `
    -serial stdio `
    -no-reboot
