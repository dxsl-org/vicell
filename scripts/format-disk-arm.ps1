# format-disk-arm.ps1 — Build aarch64 cells and create disk_arm_virt.img
#
# Produces a 64 MiB FAT32 image for the QEMU ARM virt machine.
# Analogous to format-disk.ps1 but targets aarch64-unknown-none-softfloat.
#
# Prerequisites (on the build host):
#   - mtools  (mcopy, mformat, mpartition) — available via msys2 or WSL
#   - rustup target add aarch64-unknown-none-softfloat
#
# Usage: .\scripts\format-disk-arm.ps1 [-OutFile disk_arm_virt.img] [-SizeMiB 64]

param(
    [string]$OutFile = "disk_arm_virt.img",
    [int]$SizeMiB   = 64,
    [string]$Target  = "aarch64-unknown-none-softfloat",
    [string]$Profile = "release"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$BinDir     = "target\$Target\$Profile"
$StagingDir = ".disk-staging-arm"

# ----- Step 1: Build all cells for aarch64 -----
Write-Host "[format-disk-arm] Building aarch64 cells ($Profile)..."
$buildArgs = @(
    "--release",
    "--target", $Target,
    "-p", "app-init",
    "-p", "app-shell",
    "-p", "service-vfs",
    "-p", "service-config",
    "-p", "service-net",
    "-p", "app-robot-demo",
    "-p", "periph-demo",
    "-p", "sensor-demo",
    "-p", "spi-demo"
)
# Optional cells — skip if crate not present
$optionalArgs = @()
foreach ($pkg in @("service-input", "service-compositor")) {
    $found = cargo metadata --no-deps --format-version 1 2>$null |
             Select-String -Pattern "`"name`":`"$pkg`"" -Quiet
    if ($found) { $optionalArgs += "-p"; $optionalArgs += $pkg }
}
if ($optionalArgs.Count -gt 0) { $buildArgs += $optionalArgs }

cargo build @buildArgs 2>&1 | Select-Object -Last 10

# ----- Step 2: Create blank image -----
Write-Host "[format-disk-arm] Creating $OutFile ($SizeMiB MiB)..."
$bytes = $SizeMiB * 1024 * 1024
$fs = [System.IO.File]::Create($OutFile)
$fs.SetLength($bytes)
$fs.Close()

# ----- Step 3: Partition + format (requires mtools) -----
Write-Host "[format-disk-arm] Partitioning and formatting as FAT32..."
& mpartition -I -B -T 0x0b -b 2048 "$OutFile"
& mformat -i "${OutFile}@@1M" -F -v ViCellARM ::

# ----- Step 4: Create directory structure -----
Write-Host "[format-disk-arm] Creating /bin, /etc, /tmp..."
& mmd -i "${OutFile}@@1M" ::/bin
& mmd -i "${OutFile}@@1M" ::/etc
& mmd -i "${OutFile}@@1M" ::/tmp

# ----- Step 5: Populate /etc -----
New-Item -ItemType Directory -Force $StagingDir | Out-Null
Set-Content -Path "$StagingDir\hostname" -Value "ViCell-ARM" -NoNewline -Encoding ascii
& mcopy -i "${OutFile}@@1M" "$StagingDir\hostname" ::/etc/hostname

# ----- Step 6: Populate /bin/ -----
Write-Host "[format-disk-arm] Copying aarch64 cell binaries to /bin/..."

# Map cargo binary name → /bin/<name>
$cells = @(
    @{ Bin = "app-init";          Dst = "init"        },
    @{ Bin = "app-shell";         Dst = "shell"       },
    @{ Bin = "service-vfs";       Dst = "vfs"         },
    @{ Bin = "service-config";    Dst = "config"      },
    @{ Bin = "service-net";       Dst = "net"         },
    @{ Bin = "app-robot-demo";    Dst = "robot-demo"   },
    @{ Bin = "periph-demo";       Dst = "periph-demo"  },
    @{ Bin = "sensor-demo";       Dst = "sensor-demo"  },
    @{ Bin = "spi-demo";          Dst = "spi-demo"     },
    @{ Bin = "service-input";     Dst = "input"       },
    @{ Bin = "service-compositor";Dst = "compositor"  }
)

foreach ($c in $cells) {
    $src = "$BinDir\$($c.Bin)"
    if (Test-Path $src) {
        $kb = [Math]::Round((Get-Item $src).Length / 1KB, 0)
        Write-Host "  /bin/$($c.Dst) <- $src (${kb} KB)"
        & mcopy -i "${OutFile}@@1M" $src "::/bin/$($c.Dst)"
    } else {
        Write-Warning "  [skip] $($c.Dst): $src not found"
    }
}

# ----- Cleanup -----
if (Test-Path $StagingDir) { Remove-Item -Recurse -Force $StagingDir }

Write-Host ""
Write-Host "[format-disk-arm] Done: $OutFile"
Write-Host ""
Write-Host "Next: build the aarch64 kernel and run:"
Write-Host "  `$env:RUSTFLAGS = '-C relocation-model=pic'"
Write-Host "  cargo build --release -p vicell-kernel --target aarch64-unknown-none-softfloat"
Write-Host "  `$env:RUSTFLAGS = `$null"
Write-Host "  .\run-arm-virt.ps1"
Write-Host ""
Write-Host "MQTT broker on host (port 11883, forwarded from guest port 1883):"
Write-Host "  mosquitto -p 11883"
Write-Host "  mosquitto_sub -p 11883 -t 'vios/#' -v"
