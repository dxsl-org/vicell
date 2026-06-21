# Run ViCell RV32 Nano on QEMU riscv32 virt + OpenSBI (Phase 31 smoke boot)
#
# Expected output:
#   [ViCell] kernel boot v0.2.0
#   Kernel started (Hart: 0, DTB: ...)
#   [boot] kernel_phys_base=0x...
#   [boot] Paging: bare physical (SATP=0, Phase-31 Nano)
#   [INFO] Kernel initialization complete. Entering idle loop.
#
# Usage:  .\run-rv32-virt.ps1
#         .\run-rv32-virt.ps1 -Build     # also re-builds the kernel first

param(
    [switch]$Build
)

$KernelElf = "target\riscv32imac-unknown-none-elf\release\vicell-kernel"

if ($Build) {
    Write-Host "[run-rv32] Building kernel..."
    $env:RUSTFLAGS = ""   # non-PIE; no -Crelocation-model=pic for kernel
    cargo build -p vicell-kernel --target riscv32imac-unknown-none-elf --release
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

if (-not (Test-Path $KernelElf)) {
    Write-Error "Kernel ELF not found: $KernelElf  (run with -Build first)"
    exit 1
}

Write-Host "[run-rv32] Starting QEMU riscv32 virt..."
& "C:\Program Files\qemu\qemu-system-riscv32.exe" `
    -machine virt `
    -cpu rv32 `
    -m 256M `
    -bios default `
    -kernel $KernelElf `
    -nographic `
    -serial mon:stdio
