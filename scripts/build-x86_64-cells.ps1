# build-x86_64-cells.ps1 — Build x86_64 cells and create kernel_fs.img for x86_64
#
# Builds app-shell and service-vfs (+ service-config) for x86_64-unknown-none,
# then packages them into kernel/src/embedded-x86_64/kernel_fs.img.
#
# Run from the ViCell root directory.

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$target    = "x86_64-unknown-none"
$buildDir  = "target\$target\release"
$embedded  = "kernel\src\embedded-x86_64"
$buildStd  = "-Z build-std=core,alloc"
$rustflags = "-C relocation-model=static -C target-feature=-red-zone"

Write-Host "=== Building x86_64 cells (release) ==="

$env:RUSTFLAGS = $rustflags

# Build shell
Write-Host "Building app-shell..."
$cmd = "cargo build --release -p app-shell --target $target $buildStd 2>&1"
Invoke-Expression $cmd | Select-Object -Last 10
if ($LASTEXITCODE -ne 0) { Write-Warning "app-shell build failed (exit $LASTEXITCODE)" }

# Build vfs
Write-Host "Building service-vfs..."
$cmd = "cargo build --release -p service-vfs --target $target $buildStd 2>&1"
Invoke-Expression $cmd | Select-Object -Last 10
if ($LASTEXITCODE -ne 0) { Write-Warning "service-vfs build failed (exit $LASTEXITCODE)" }

# Build config
Write-Host "Building service-config..."
$cmd = "cargo build --release -p service-config --target $target $buildStd 2>&1"
Invoke-Expression $cmd | Select-Object -Last 10
if ($LASTEXITCODE -ne 0) { Write-Warning "service-config build failed (exit $LASTEXITCODE)" }

$env:RUSTFLAGS = ""

# Collect available binaries
$cells = @(
    @{ Bin = "app-shell";      Dst = "/bin/shell"  },
    @{ Bin = "service-vfs";    Dst = "/bin/vfs"    },
    @{ Bin = "service-config"; Dst = "/bin/config" }
)

$imgArgs = @("kernel\src\embedded-x86_64\kernel_fs.img")
$found   = @()
foreach ($c in $cells) {
    $src = "$buildDir\$($c.Bin)"
    if (Test-Path $src) {
        $kb = [Math]::Round((Get-Item $src).Length / 1KB, 0)
        Write-Host "  Found: $($c.Bin) (${kb} KB) -> $($c.Dst)"
        $imgArgs += "$src`:$($c.Dst)"
        $found += $c.Bin
    } else {
        Write-Warning "  Not found: $src (will be absent from kernel_fs.img)"
    }
}

if ($found.Count -eq 0) {
    Write-Error "No x86_64 cell binaries built — kernel_fs.img not updated."
    exit 1
}

Write-Host ""
Write-Host "=== Creating x86_64 kernel_fs.img ==="
python tools\mkfat32.py @imgArgs
if ($LASTEXITCODE -ne 0) {
    Write-Error "mkfat32.py failed (exit $LASTEXITCODE)"
    exit 1
}
$kb = [Math]::Round((Get-Item "kernel\src\embedded-x86_64\kernel_fs.img").Length / 1KB, 0)
Write-Host "  kernel_fs.img created: ${kb} KB"

Write-Host ""
Write-Host "Done. Rebuild kernel to embed the new x86_64 cells:"
Write-Host "  cargo build --release -p vicell-kernel --target x86_64-unknown-none -Z build-std=core,alloc"
