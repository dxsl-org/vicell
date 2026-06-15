# run-hypervisor-arm.ps1 — Boot ViCell on QEMU ARM virt (aarch64) with EL2 hypervisor.
#
# Boots the ViCell kernel at EL2, which then runs the hypervisor cell to launch
# an Alpine Linux guest via Stage-2 MMU (Tier 3b — ARM64 EL2 VMM).
#
# Prerequisites:
#   1. qemu-system-aarch64 >= 8.0 (for cortex-a72 + virtualization=on support).
#      Install via winget: winget install QEMU.QEMU
#      Or QEMU installer: https://www.qemu.org/download/
#   2. Build the aarch64 kernel WITH the Alpine guest image embedded (see below):
#        .\scripts\make-hypervisor-fs.ps1       # fetch Alpine + build kernel_fs_hv.img
#        $env:RUSTFLAGS = "-C relocation-model=pic"
#        $env:EMBEDDED_OVERRIDE = "kernel\src\embedded-hv"
#        cargo build --release -p vicell-kernel --target aarch64-unknown-none-softfloat
#        $env:RUSTFLAGS  = $null
#        $env:EMBEDDED_OVERRIDE = $null
#   3. Build the hypervisor disk image:
#        .\scripts\format-disk-hv-arm.ps1
#
# KVM acceleration (real ARM64 host only — e.g. RK3588, Raspberry Pi 5):
#   Add -enable-kvm to the qemu args below for near-native guest performance.
#   CI runners are x86 and use TCG (software emulation); KVM is not available there.
#
# Guest networking:
#   Alpine gets 10.0.2.15 via SLIRP DHCP through the Net Cell.
#   The guest can reach the internet via SLIRP user-mode networking.
#   Port 2222 on the host is forwarded to port 22 in the guest for SSH.

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
$disk    = "disk_hv_arm.img"

if (-not (Test-Path $kernel)) {
    Write-Host "Hypervisor kernel not found: $kernel"
    Write-Host "Build it with:"
    Write-Host "  .\scripts\make-hypervisor-fs.ps1"
    Write-Host "  `$env:RUSTFLAGS = '-C relocation-model=pic'"
    Write-Host "  `$env:EMBEDDED_OVERRIDE = 'kernel\src\embedded-hv'"
    Write-Host "  cargo build --release -p vicell-kernel --target $target"
    Write-Host "  `$env:RUSTFLAGS = `$null; `$env:EMBEDDED_OVERRIDE = `$null"
    exit 1
}

if (-not (Test-Path $disk)) {
    Write-Host "Hypervisor disk image not found: $disk"
    Write-Host "Build it with: .\scripts\format-disk-hv-arm.ps1"
    exit 1
}

Write-Host ""
Write-Host "Starting ViCell hypervisor on QEMU ARM virt (aarch64 EL2)..."
Write-Host "  Machine:  virt,virtualization=on,gic-version=2"
Write-Host "  CPU:      cortex-a72 (ARMv8.0, EL2 capable)"
Write-Host "  RAM:      1 GiB (512 MiB host ViCell + 128 MiB guest Alpine)"
Write-Host "  Guest:    Alpine Linux via Stage-2 MMU (Tier 3b VMM)"
Write-Host ""
Write-Host "Wait for 'ViCell >' shell, then Alpine boots automatically via /bin/hypervisor."
Write-Host "Inside Alpine guest, you should see '/ #' prompt after DHCP."
Write-Host "Press Ctrl-a x to quit QEMU."
Write-Host ""

& $qemu `
    -machine virt,virtualization=on,gic-version=2 `
    -cpu cortex-a72 `
    -m 1G `
    -nographic `
    -kernel $kernel `
    -drive if=none,file=$disk,format=raw,id=hd0 `
    -device virtio-blk-device,drive=hd0 `
    -netdev user,id=net0,hostfwd=tcp::2222-:22 `
    -device virtio-net-device,netdev=net0 `
    -no-reboot
