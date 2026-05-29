# Run ViOS in QEMU
$qemu = "qemu-system-riscv64"
if (Get-Command $qemu -ErrorAction SilentlyContinue) {
    # QEMU in PATH
} elseif (Test-Path "C:\Program Files\qemu\qemu-system-riscv64.exe") {
    $qemu = "C:\Program Files\qemu\qemu-system-riscv64.exe"
} else {
    Write-Host "QEMU not found. Please install QEMU or add it to PATH."
    exit 1
}

# Release kernel now only 4.4 MB (kernel_fs.img embedded separately).
# 256 MB is sufficient: kernel(4.4MB) + heap(64MB) + cells + stacks.
$kernel = "target/riscv64gc-unknown-none-elf/release/vios-kernel"
$disk   = "disk_v3.img"

# Build release kernel if not present
if (-not (Test-Path $kernel)) {
    Write-Host "Release kernel not found — building..."
    cargo build --release -p vios-kernel
}

Write-Host "Starting ViOS in QEMU (Nographic Mode)..."
Write-Host "Tip: Press 'Ctrl-a' then 'x' to exit QEMU."
Write-Host "Boot: OpenSBI → kernel (4.4MB) → init → VFS → config → shell (ViOS>)"
Write-Host ""

# kernel_fs.img (4 MB FAT32 with release cells) is embedded in the kernel binary.
# disk_v3.img (40 MB blank + bootstrap table) is the VirtIO block disk.
& $qemu -machine virt -m 256M -nographic -bios default -kernel $kernel `
        -drive file=$disk,format=raw,id=hd0,if=none `
        -device virtio-blk-device,drive=hd0
