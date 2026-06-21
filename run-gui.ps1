# Run ViCell in QEMU with graphical display (VirtIO GPU → compositor → ViUI).
$qemu = "qemu-system-riscv64"
if (Get-Command $qemu -ErrorAction SilentlyContinue) {
    # QEMU in PATH
} elseif (Test-Path "C:\Program Files\qemu\qemu-system-riscv64.exe") {
    $qemu = "C:\Program Files\qemu\qemu-system-riscv64.exe"
} else {
    Write-Host "QEMU not found. Please install QEMU or add it to PATH."
    exit 1
}

$kernel = "target/riscv64gc-unknown-none-elf/release/vicell-kernel"
$disk   = "disk_v3.img"

Write-Host "Building release kernel..."
$env:RUSTFLAGS = "-C relocation-model=pic"
cargo build --release -p vicell-kernel
$env:RUSTFLAGS = $null
if (-not (Test-Path $kernel)) { Write-Host "Kernel build failed."; exit 1 }

Write-Host "Starting ViCell in Graphical Mode (compositor → VirtIO GPU)..."
Write-Host ""
Write-Host "  QEMU window  →  compositor / ViUI graphical output (GPU framebuffer)"
Write-Host "  THIS terminal →  shell (ViCell>) + kernel logs  (UART serial)"
Write-Host ""
Write-Host "Shell is HERE in this terminal, not in the QEMU window."
Write-Host "Type commands HERE after the 'ViCell >' prompt appears."
Write-Host "Typing into the QEMU window goes to GUI apps (compositor), NOT the shell,"
Write-Host "and QEMU's GTK keymap on Windows may emit 'unmapped key' noise → see qemu-host.log."
Write-Host ""

# QEMU host diagnostics (audio backend, GTK keymap 'unmapped key', etc.) go to
# STDERR; redirect them to a log file so the shell on STDOUT stays clean. The
# serial console (shell + kernel) is on STDOUT and stays in this terminal.
& $qemu -machine virt -m 256M -bios default -kernel $kernel `
    -drive "file=$disk,format=raw,id=hd0,if=none" `
    -device virtio-blk-device,drive=hd0 `
    -netdev user,id=net0 `
    -device virtio-net-device,netdev=net0 `
    -device virtio-gpu-device `
    -device virtio-keyboard-device `
    -device virtio-mouse-device `
    -audiodev "wav,id=snd0,path=vicell-audio.wav" `
    -device virtio-sound-device,audiodev=snd0 `
    -display gtk `
    -serial stdio 2> "qemu-host.log"
