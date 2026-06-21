# setup-mlibc.ps1 — Build mlibc libc.a on Windows (no WSL2 required).
#
# What this script does:
#   1. Clones managarm/mlibc into third_party/mlibc-src/ (skips if already cloned)
#   2. Copies ViCell's sysdeps/vicell into the clone
#   3. Patches mlibc's meson.build to add the 'vicell' host_machine.system() branch
#   4. Runs meson setup + ninja to build libc.a for riscv64
#   5. Copies the resulting libc.a to third_party/mlibc/build/libc.a
#
# Requirements (all checked below):
#   - git
#   - meson  (pip install meson)
#   - ninja  (ships with WinLibs / install via winget install Ninja-build.Ninja)
#   - riscv-none-elf-gcc  (xpack at C:\RISCV\xpack-riscv-none-elf-gcc-15.2.0-1)
#
# aarch64 still requires WSL2 (no Windows aarch64-none-elf toolchain included).
# Run `bash scripts/build-mlibc.sh` in WSL2 to also produce build-aarch64/libc.a.

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$RepoRoot   = Split-Path $PSScriptRoot -Parent
$MlibcSrc   = Join-Path $RepoRoot 'third_party\mlibc-src'
$OurSysdeps = Join-Path $RepoRoot 'third_party\mlibc\sysdeps\vicell'
$BuildOut   = Join-Path $RepoRoot 'third_party\mlibc\build'
$CrossFile  = Join-Path $RepoRoot 'scripts\mlibc-riscv64-windows.cross'
$MlibcRepo  = 'https://github.com/managarm/mlibc.git'

function Check-Command($cmd) {
    if (-not (Get-Command $cmd -ErrorAction SilentlyContinue)) {
        Write-Error "Required tool not found: $cmd`nInstall it and re-run this script."
        exit 1
    }
    Write-Host "  ✓ $cmd found" -ForegroundColor Green
}

Write-Host "`n=== Checking prerequisites ===" -ForegroundColor Cyan
Check-Command git
Check-Command meson
Check-Command ninja
Check-Command riscv-none-elf-gcc

# ─── Step 1: Clone mlibc source ───────────────────────────────────────────────
Write-Host "`n=== Step 1: mlibc source ===" -ForegroundColor Cyan
if (Test-Path (Join-Path $MlibcSrc '.git')) {
    Write-Host "  mlibc already cloned at $MlibcSrc — pulling latest..." -ForegroundColor Yellow
    git -C $MlibcSrc pull --ff-only
} else {
    Write-Host "  Cloning $MlibcRepo ..."
    New-Item -ItemType Directory -Force $MlibcSrc | Out-Null
    git clone --depth 1 $MlibcRepo $MlibcSrc
}

# Record the commit SHA for reproducibility
$mlibcSha = git -C $MlibcSrc rev-parse HEAD
Write-Host "  mlibc commit: $mlibcSha" -ForegroundColor DarkGray

# ─── Step 2: Install ViCell sysdeps ───────────────────────────────────────────
Write-Host "`n=== Step 2: Installing ViCell sysdeps ===" -ForegroundColor Cyan
$destSysdeps = Join-Path $MlibcSrc 'sysdeps\vicell'
if (-not (Test-Path $destSysdeps)) {
    New-Item -ItemType Directory -Force $destSysdeps | Out-Null
}
Copy-Item -Recurse -Force "$OurSysdeps\*" $destSysdeps
Write-Host "  Copied sysdeps/vicell → $destSysdeps" -ForegroundColor Green

# ─── Step 3: Patch mlibc's meson.build ────────────────────────────────────────
Write-Host "`n=== Step 3: Patching meson.build ===" -ForegroundColor Cyan
$mesonBuild = Join-Path $MlibcSrc 'meson.build'
$content    = Get-Content $mesonBuild -Raw

$patch = @"

elif host_machine.system() == 'vicell'
    subdir('sysdeps/vicell')
"@

# Only patch if the vicell branch isn't already there
if ($content -notmatch "host_machine\.system\(\) == 'vicell'") {
    # Insert before the closing else/error in the host_machine.system() dispatch block.
    # managarm/mlibc uses a pattern like: elif host_machine.system() == '<os>'
    # followed eventually by: else / error(...)
    # We insert before the final else that terminates the chain.
    $content = $content -replace "(elif host_machine\.system\(\) == '[^']+'\s*\n\s*subdir\('[^']+'\)\s*\n)(else\s*\n\s*error\()", "`$1$patch`n`$2"

    if ($content -match "host_machine\.system\(\) == 'vicell'") {
        Set-Content -Path $mesonBuild -Value $content -NoNewline
        Write-Host "  meson.build patched successfully" -ForegroundColor Green
    } else {
        Write-Warning "  Auto-patch failed — inserting at end of system() dispatch manually"
        # Fallback: append before the last else..error block
        $content = $content -replace "(else\s*\n\s*error\('Unknown OS)", "$patch`n`$1"
        Set-Content -Path $mesonBuild -Value $content -NoNewline
        Write-Host "  meson.build patched (fallback method)" -ForegroundColor Yellow
    }
} else {
    Write-Host "  meson.build already has vicell branch — skipping" -ForegroundColor DarkGray
}

# ─── Step 4: meson setup ──────────────────────────────────────────────────────
Write-Host "`n=== Step 4: meson setup (riscv64) ===" -ForegroundColor Cyan
$buildDir = Join-Path $MlibcSrc 'build-win'

$mesonArgs = @(
    'setup', $buildDir, $MlibcSrc,
    "--cross-file=$CrossFile",
    '-Ddefault_library=static',
    '-Dposix_option=disabled',
    '-Dlinux_option=disabled',
    '-Dheaders_only=false',
    '--wipe'
)

Write-Host "  meson $($mesonArgs -join ' ')"
& meson @mesonArgs
if ($LASTEXITCODE -ne 0) { Write-Error "meson setup failed"; exit 1 }

# ─── Step 5: ninja build ──────────────────────────────────────────────────────
Write-Host "`n=== Step 5: ninja build ===" -ForegroundColor Cyan
& ninja -C $buildDir
if ($LASTEXITCODE -ne 0) { Write-Error "ninja build failed"; exit 1 }

# ─── Step 6: copy libc.a to expected location ────────────────────────────────
Write-Host "`n=== Step 6: Installing libc.a ===" -ForegroundColor Cyan
New-Item -ItemType Directory -Force $BuildOut | Out-Null
$srcLib  = Join-Path $buildDir 'libc.a'
$destLib = Join-Path $BuildOut 'libc.a'
Copy-Item -Force $srcLib $destLib

$size = (Get-Item $destLib).Length / 1KB
Write-Host "  third_party/mlibc/build/libc.a  ($([int]$size) KB)" -ForegroundColor Green

Write-Host "`n✅ mlibc riscv64 build complete!" -ForegroundColor Green
Write-Host "   Run 'cargo check' — mlibc-shim warning should be gone."
Write-Host "   For aarch64: run 'bash scripts/build-mlibc.sh' in WSL2."
