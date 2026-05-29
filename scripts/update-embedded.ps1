# update-embedded.ps1 — Build cells in release mode and install to kernel/src/embedded/
#
# The kernel embeds init, vfs, shell, and config via include_bytes!.
# Debug builds are ~3-5x larger than release builds.  Using release binaries
# dramatically reduces the kernel binary size:
#   Debug: kernel ~52 MB (debug cells embedded in rodata)
#   Release cells: kernel ~15 MB
#
# Run this after any change to the embedded cells.

param(
    [switch]$Debug  # use debug builds (default: release)
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$target = "target\riscv64gc-unknown-none-elf"
$buildProfile = if ($Debug) { "debug" } else { "release" }
$binDir  = "$target\$buildProfile"
$embedded = "kernel\src\embedded"

Write-Host "Building embedded cells in $buildProfile mode..."
$buildArgs = @("-p", "app-init", "-p", "app-shell", "-p", "service-vfs", "-p", "service-config")
if (-not $Debug) { $buildArgs = @("--release") + $buildArgs }
cargo build @buildArgs 2>&1 | Select-Object -Last 5

# Map binary name → embedded filename
$cells = @(
    @{ Bin = "app-init";       Dst = "init"   },
    @{ Bin = "app-shell";      Dst = "shell"  },
    @{ Bin = "service-vfs";    Dst = "vfs"    },
    @{ Bin = "service-config"; Dst = "config" }
)

# Lua is always release-only (C build)
$luaBin = "$target\release\lua"
if (Test-Path $luaBin) {
    $cells += @{ Bin = "lua"; Dst = "lua"; FullPath = $luaBin }
}

Write-Host ""
Write-Host "Installing to $embedded/:"
foreach ($c in $cells) {
    $src = if ($c.ContainsKey('FullPath')) { $c.FullPath } else { "$binDir\$($c.Bin)" }
    if (Test-Path $src) {
        Copy-Item -Path $src -Destination "$embedded\$($c.Dst)" -Force
        $kb = [Math]::Round((Get-Item $src).Length / 1KB, 0)
        Write-Host "  $($c.Dst) <- $src (${kb} KB)"
    } else {
        Write-Warning "  $($c.Dst): $src not found — skipping"
    }
}

Write-Host ""
Write-Host "Done. Rebuild the kernel to embed the new cells:"
Write-Host "  cargo build --release -p vios-kernel"
