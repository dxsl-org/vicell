# format-disk.ps1 — Generate the ViCell disk image with FAT32 and populate /bin/
#
# Prerequisites (on the build host):
#   - mtools  (mcopy, mformat, mpartition) — available via msys2 or WSL
#   - Built cell binaries under target/riscv64gc-unknown-none-elf/release/
#
# Output: disk.img (64 MiB, MBR partition table, FAT32 primary partition)
#
# Usage: .\scripts\format-disk.ps1 [-OutFile disk.img] [-SizeMiB 64]

param(
    [string]$OutFile = "disk.img",
    [int]$SizeMiB   = 64,
    [string]$Target = "riscv64gc-unknown-none-elf",
    [string]$Profile = "release"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$BinDir     = "target\$Target\$Profile"
$StagingDir = ".disk-staging"

# ----- Step 1: Create blank image -----
Write-Host "[format-disk] Creating $OutFile ($SizeMiB MiB)..."
$bytes = $SizeMiB * 1024 * 1024
$fs = [System.IO.File]::Create($OutFile)
$fs.SetLength($bytes)
$fs.Close()

# ----- Step 2: Partition + format (requires mtools) -----
# mpartition: create MBR with a single FAT32 primary partition starting at 1 MiB
Write-Host "[format-disk] Partitioning and formatting as FAT32..."
& mpartition -I -B -T 0x0b -b 2048 "$OutFile"   # MBR + type 0x0b = FAT32
& mformat -i "${OutFile}@@1M" -F -v ViCell ::       # FAT32 volume label ViCell

# ----- Step 3: Create directory structure -----
Write-Host "[format-disk] Creating directory structure..."
& mmd -i "${OutFile}@@1M" ::/bin
& mmd -i "${OutFile}@@1M" ::/etc
& mmd -i "${OutFile}@@1M" ::/tmp

# ----- Step 4: Populate /etc -----
Write-Host "[format-disk] Writing /etc/hostname..."
"ViCell" | Out-File -Encoding ascii -NoNewline -FilePath "$StagingDir\hostname"
New-Item -ItemType Directory -Force $StagingDir | Out-Null
Set-Content -Path "$StagingDir\hostname" -Value "ViCell" -NoNewline -Encoding ascii
& mcopy -i "${OutFile}@@1M" "$StagingDir\hostname" ::/etc/hostname

# ----- Step 5: Populate /bin/ from build output -----
Write-Host "[format-disk] Copying cell binaries to /bin/..."
$cells = @("init", "config", "shell", "vfs", "hello", "echo", "cat", "ls", "lua")
foreach ($name in $cells) {
    $src = "$BinDir\$name"
    if (Test-Path $src) {
        Write-Host "  $src -> /bin/$name"
        & mcopy -i "${OutFile}@@1M" $src "::/bin/$name"
    } else {
        Write-Warning "  [skip] $src not found — run 'cargo build --release' first"
    }
}

# ----- Cleanup -----
if (Test-Path $StagingDir) { Remove-Item -Recurse -Force $StagingDir }

Write-Host "[format-disk] Done: $OutFile"
Write-Host ""
Write-Host "To use with QEMU:"
Write-Host "  qemu-system-riscv64 -drive file=$OutFile,format=raw,if=virtio ..."
