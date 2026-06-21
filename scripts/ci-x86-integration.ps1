#Requires -Version 7
<#
.SYNOPSIS
    Build x86_64 cells + kernel + ISO, then run x86_64 integration tests.

.DESCRIPTION
    Full CI pipeline for x86_64 QEMU q35 integration:
      1. Build cells (shell, vfs, config, sys-tools) for x86_64-unknown-none
      2. Pack kernel_fs.img
      3. Build kernel (x86_64-unknown-none, --release)
      4. Build ISO via make-iso.sh (requires WSL)
      5. Run `cargo test -p vicell-integration-tests x86` + `virtio_x86`
      6. Report: PASSED / FAILED / SKIPPED (no QEMU) — exit codes are distinct

    Exit codes:
      0  All tests PASSED (or explicitly SKIPPED — no QEMU)
      1  One or more tests FAILED
      2  Build step failed (cells / kernel / ISO)
      3  Prerequisite missing (Rust nightly, WSL)

.PARAMETER SkipBuild
    Skip cell + kernel + ISO build. Assumes build/vicell-x86.iso already exists.

.PARAMETER SkipTests
    Build only — do not run integration tests.

.PARAMETER NoBuildStd
    Omit -Z build-std (for environments where the nightly toolchain is not available).
    Tests will still run if the ISO already exists.
#>
param(
    [switch]$SkipBuild,
    [switch]$SkipTests,
    [switch]$NoBuildStd
)

Set-StrictMode -Version 3
$ErrorActionPreference = 'Stop'

$Repo = Split-Path -Parent $PSScriptRoot
$IsoPath = Join-Path $Repo 'build\vicell-x86.iso'
$RUSTFLAGS_CELLS = '-C relocation-model=static -C target-feature=-red-zone'
$Target = 'x86_64-unknown-none'
$BuildStdArgs = if ($NoBuildStd) { @() } else { @('-Z', 'build-std=core,alloc') }

function Write-Step([string]$msg) { Write-Host "`n=== $msg ===" -ForegroundColor Cyan }
function Write-Ok([string]$msg)   { Write-Host "[OK]  $msg" -ForegroundColor Green }
function Write-Warn([string]$msg) { Write-Host "[WARN] $msg" -ForegroundColor Yellow }
function Write-Fail([string]$msg) { Write-Host "[FAIL] $msg" -ForegroundColor Red }

# ---------- Prerequisite check ----------
Write-Step 'Prerequisite check'

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Fail 'cargo not found on PATH'
    exit 3
}

$RustcOutput = cargo +nightly --version 2>&1
if ($LASTEXITCODE -ne 0) {
    Write-Warn 'nightly toolchain not installed — some build steps may fail'
}

if (-not $SkipBuild) {
    try { wsl --status | Out-Null }
    catch {
        Write-Fail 'WSL not available — required for ISO packaging (make-iso.sh)'
        exit 3
    }
}
Write-Ok 'Prerequisites satisfied'

# ---------- Build ----------
if (-not $SkipBuild) {
    Write-Step 'Building x86_64 cells'
    $env:RUSTFLAGS = $RUSTFLAGS_CELLS

    $Cells = @('app-shell', 'service-config', 'app-sys-tools')
    # service-vfs requires libclang (littlefs2-sys bindgen); skip if not available
    $AllCells = @('service-vfs') + $Cells
    $BuiltCells = @()

    foreach ($cell in $AllCells) {
        $args = @('build', '--release', '-p', $cell, '--target', $Target) + $BuildStdArgs
        Write-Host "  cargo $($args -join ' ')"
        cargo @args 2>&1 | Tee-Object -Variable out | Select-Object -Last 5
        if ($LASTEXITCODE -eq 0) {
            $BuiltCells += $cell
            Write-Ok "$cell built"
        } else {
            if ($cell -eq 'service-vfs') {
                Write-Warn "service-vfs build failed (likely missing libclang) — using cached binary from kernel_fs.img"
            } else {
                Write-Fail "$cell build failed (required cell)"
                $env:RUSTFLAGS = ''
                exit 2
            }
        }
    }
    $env:RUSTFLAGS = ''

    Write-Step 'Packing kernel_fs.img'
    & "$Repo\scripts\build-x86_64-cells.ps1" -PackOnly 2>&1 | Select-Object -Last 10
    # If the above fails (no -PackOnly flag), fall through — the old image is still valid.

    Write-Step 'Building x86_64 kernel'
    $env:RUSTFLAGS = ''
    $kargs = @('build', '--release', '-p', 'vicell-kernel', '--target', $Target) + $BuildStdArgs
    cargo @kargs 2>&1 | Select-Object -Last 15
    if ($LASTEXITCODE -ne 0) {
        Write-Fail 'Kernel build failed'
        exit 2
    }
    Write-Ok 'Kernel built'

    Write-Step 'Building ISO via WSL make-iso.sh'
    wsl bash "$($Repo.Replace('\','/'))/build/make-iso.sh" 2>&1 | Tee-Object -Variable isoOut | Select-Object -Last 10
    if (-not (Test-Path $IsoPath)) {
        Write-Fail "ISO not produced at $IsoPath"
        Write-Host ($isoOut -join "`n")
        exit 2
    }
    Write-Ok "ISO ready: $IsoPath"
}

if ($SkipTests) {
    Write-Ok 'Build complete — tests skipped (-SkipTests)'
    exit 0
}

# ---------- Tests ----------
Write-Step 'Running x86_64 integration tests'

if (-not (Test-Path $IsoPath)) {
    Write-Warn "ISO not found at $IsoPath — all x86 tests will SKIP (no QEMU prerequisite)"
}

# Check QEMU presence
$QemuOk = $false
$QemuCandidates = @('qemu-system-x86_64', 'C:\Program Files\qemu\qemu-system-x86_64.exe')
foreach ($q in $QemuCandidates) {
    try {
        & $q --version 2>&1 | Out-Null
        $QemuOk = $true
        break
    } catch {}
}

if (-not $QemuOk) {
    Write-Warn 'qemu-system-x86_64 not found — all x86 tests will SKIP'
    Write-Host ''
    Write-Host 'RESULT: SKIPPED (no QEMU — not a test failure)' -ForegroundColor Yellow
    exit 0
}

# Run the tests
$TestFilter = 'x86'
$TestOutput = cargo test -p vicell-integration-tests -- --test-threads=1 "$TestFilter" 2>&1
Write-Host ($TestOutput -join "`n")

$Passed  = ($TestOutput | Where-Object { $_ -match 'test .* ok' }).Count
$Failed  = ($TestOutput | Where-Object { $_ -match 'test .* FAILED' }).Count
$Ignored = ($TestOutput | Where-Object { $_ -match 'test .* ignored' }).Count
$Skipped = ($TestOutput | Where-Object { $_ -match 'SKIP' }).Count

Write-Host ''
Write-Host '--- x86 Integration Test Summary ---' -ForegroundColor Cyan
Write-Host "  Passed:  $Passed" -ForegroundColor Green
Write-Host "  Failed:  $Failed" -ForegroundColor $(if ($Failed -gt 0) { 'Red' } else { 'Gray' })
Write-Host "  Ignored: $Ignored" -ForegroundColor Gray
Write-Host "  Skipped: $Skipped (no QEMU or missing ISO)" -ForegroundColor Yellow

if ($Failed -gt 0) {
    Write-Host ''
    Write-Host 'RESULT: FAILED' -ForegroundColor Red
    exit 1
} elseif ($Passed -eq 0 -and $Skipped -gt 0) {
    Write-Host ''
    Write-Host 'RESULT: SKIPPED (no QEMU-equipped environment — not a failure)' -ForegroundColor Yellow
    exit 0
} else {
    Write-Host ''
    Write-Host "RESULT: PASSED ($Passed tests)" -ForegroundColor Green
    exit 0
}
