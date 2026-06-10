# dev-setup.ps1 — One-command ViCell development environment setup for Windows.
#
# Supports: Windows 10 (21H2+) / Windows 11, PowerShell 7+
# Idempotent: safe to run multiple times.
#
# Usage:
#   .\scripts\dev-setup.ps1          # install everything
#   .\scripts\dev-setup.ps1 --check  # verify existing install without installing
#   .\scripts\dev-setup.ps1 --help   # show this help
#
# Requires: winget (ships with Windows 11; install App Installer on Win10)

param(
    [switch]$Check,
    [switch]$Help
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Write-Step  { param([string]$Msg) Write-Host "`n=== $Msg ===" -ForegroundColor Cyan }
function Write-Info  { param([string]$Msg) Write-Host "[setup] $Msg" -ForegroundColor Green }
function Write-Warn  { param([string]$Msg) Write-Host "[warn]  $Msg" -ForegroundColor Yellow }
function Write-Fail  { param([string]$Msg) Write-Host "[error] $Msg" -ForegroundColor Red; exit 1 }

if ($Help) {
    Write-Host "Usage: .\scripts\dev-setup.ps1 [-Check] [-Help]"
    Write-Host "  -Check   Verify the environment without installing anything."
    exit 0
}

$RepoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $RepoRoot
Write-Step "ViCell Developer Setup (Windows)"
Write-Info "Repo: $RepoRoot"

# ── Read pinned toolchain ──────────────────────────────────────────────────────
$ToolchainFile = Join-Path $RepoRoot "rust-toolchain.toml"
$Toolchain = "nightly"
if (Test-Path $ToolchainFile) {
    $line = Get-Content $ToolchainFile | Where-Object { $_ -match 'channel' } | Select-Object -First 1
    if ($line -match '"([^"]+)"') { $Toolchain = $Matches[1] }
}
Write-Info "Pinned Rust toolchain: $Toolchain"

# ── 1. Rustup ─────────────────────────────────────────────────────────────────
Write-Step "1/5 Rust toolchain"
$rustupPresent = $null -ne (Get-Command rustup -ErrorAction SilentlyContinue)

if (-not $rustupPresent) {
    if ($Check) { Write-Fail "rustup not found — run without -Check to install" }
    Write-Info "Installing Rust via winget..."
    winget install --id Rustlang.Rustup --silent --accept-source-agreements --accept-package-agreements
    # Reload PATH for this session
    $env:PATH = [System.Environment]::GetEnvironmentVariable("PATH", "Machine") + ";" +
                [System.Environment]::GetEnvironmentVariable("PATH", "User")
}
Write-Info "rustup $(rustup --version 2>$null | Select-Object -First 1)"

if (-not $Check) {
    rustup toolchain install $Toolchain --allow-downgrade
    rustup component add rust-src rustfmt clippy llvm-tools-preview
    rustup target add riscv64gc-unknown-none-elf aarch64-unknown-none x86_64-unknown-none
}

# ── 2. QEMU ───────────────────────────────────────────────────────────────────
Write-Step "2/5 QEMU"
$qemuPresent = $null -ne (Get-Command qemu-system-riscv64 -ErrorAction SilentlyContinue)

if ($qemuPresent) {
    $qver = qemu-system-riscv64 --version 2>$null | Select-Object -First 1
    Write-Info "$qver ✓"
} else {
    if ($Check) {
        Write-Warn "qemu-system-riscv64 not found — QEMU boot tests will be skipped"
    } else {
        Write-Info "Installing QEMU via winget..."
        winget install --id SoftwareFreedomConservancy.QEMU `
            --silent --accept-source-agreements --accept-package-agreements
        # Reload PATH
        $env:PATH = [System.Environment]::GetEnvironmentVariable("PATH", "Machine") + ";" +
                    [System.Environment]::GetEnvironmentVariable("PATH", "User")
    }
}

# ── 3. mtools (for disk image creation) ───────────────────────────────────────
Write-Step "3/5 mtools (disk image)"
$mtoolsPresent = $null -ne (Get-Command mformat -ErrorAction SilentlyContinue)
if ($mtoolsPresent) {
    Write-Info "mtools ✓"
} elseif (-not $Check) {
    Write-Warn "mtools not found. Attempting to install via Scoop..."
    if (-not (Get-Command scoop -ErrorAction SilentlyContinue)) {
        Write-Info "Installing Scoop..."
        Set-ExecutionPolicy -ExecutionPolicy RemoteSigned -Scope CurrentUser -Force
        Invoke-RestMethod -Uri 'https://get.scoop.sh' | Invoke-Expression
    }
    scoop install mtools
} else {
    Write-Warn "mtools not found — disk image scripts require mtools or WSL"
}

# ── 4. Cargo check (smoke build) ──────────────────────────────────────────────
Write-Step "4/5 Cargo workspace check"
$checkOutput = cargo check --workspace 2>&1 | Select-Object -Last 5
Write-Host $checkOutput
if ($LASTEXITCODE -ne 0) {
    Write-Fail "cargo check failed — see output above"
}
Write-Info "cargo check ✓"

# ── 5. Summary ────────────────────────────────────────────────────────────────
Write-Step "5/5 Done!"
Write-Host ""
Write-Host "ViCell development environment is ready." -ForegroundColor Green -BackgroundColor Black
Write-Host ""
Write-Host "  Build kernel:   cargo build --release"
Write-Host "  Generate disk:  .\gen_disk.ps1"
Write-Host "  Run in QEMU:    .\run.ps1"
Write-Host "  Smoke checks:   .\scripts\check-baseline.sh  (via Git Bash / WSL)"
Write-Host ""
Write-Host "  First time?  Read docs\ONBOARDING.md"
Write-Host "  Questions?   Open a GitHub Discussion"
