#!/usr/bin/env pwsh
# ci-x86-integration.ps1 — Run the x86_64 integration test suite.
#
# Usage:
#   .\scripts\ci-x86-integration.ps1          # run all suites
#   .\scripts\ci-x86-integration.ps1 --quick  # boot + handoff only (skip nvme/nic)
#
# Exit codes:
#   0  — all tests passed (or SKIPPED because QEMU/ISO absent)
#   1  — one or more tests failed
#
# Prerequisites:
#   - qemu-system-x86_64 on PATH (or at C:\Program Files\qemu\)
#   - build/vicell-x86.iso built via scripts/build-x86_64-cells.ps1 + build/make-iso.sh

param(
    [switch]$Quick
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$iso      = Join-Path $repoRoot "build\vicell-x86.iso"

# ── Prerequisite checks ──────────────────────────────────────────────────────

$qemuFound = $false
foreach ($q in @("qemu-system-x86_64", "C:\Program Files\qemu\qemu-system-x86_64.exe")) {
    try {
        $null = & $q --version 2>$null
        $qemuFound = $true
        break
    } catch { }
}

if (-not $qemuFound) {
    Write-Host "SKIPPED: qemu-system-x86_64 not found — install QEMU to run x86 integration tests."
    exit 0
}

if (-not (Test-Path $iso)) {
    Write-Host "SKIPPED: $iso not found — build it with scripts/build-x86_64-cells.ps1 + build/make-iso.sh"
    exit 0
}

# ── Run test suites ──────────────────────────────────────────────────────────

$testDir  = Join-Path $repoRoot "tests\integration"
$failures = @()

function Invoke-Suite([string]$Name, [string]$TestBin) {
    Write-Host ""
    Write-Host "==> $Name"
    Push-Location $testDir
    try {
        cargo test --test $TestBin -- --nocapture 2>&1
        if ($LASTEXITCODE -ne 0) {
            $script:failures += $Name
            Write-Host "[FAIL] $Name exited $LASTEXITCODE"
        } else {
            Write-Host "[PASS] $Name"
        }
    } finally {
        Pop-Location
    }
}

Invoke-Suite "x86_64-boot"   "x86_64-boot"
Invoke-Suite "handoff (x86)" "handoff"

if (-not $Quick) {
    Invoke-Suite "virtio-x86" "virtio-x86"
    Invoke-Suite "nvme-x86"   "nvme-x86"
    Invoke-Suite "nic-x86"    "nic-x86"
}

# ── Summary ──────────────────────────────────────────────────────────────────

Write-Host ""
if ($failures.Count -eq 0) {
    Write-Host "All x86 integration suites passed."
    exit 0
} else {
    Write-Host "FAILED suites: $($failures -join ', ')"
    exit 1
}
