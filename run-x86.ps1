# ViCell x86_64 — build + boot via QEMU q35 with Limine bootloader
#
# Prerequisites (one-time):
#   wsl sudo apt-get install -y xorriso limine
#
# Boot goal: serial banner + "Scheduler initialized" on COM1
#   Tip: Ctrl-a x to exit QEMU (nographic mode)

param(
    [switch]$NoBuild,    # skip cargo build (reuse last ELF)
    [switch]$NoQemu      # build ISO only, do not launch QEMU
)

$ErrorActionPreference = "Stop"

# ── Paths ─────────────────────────────────────────────────────────────────────
$KERNEL_ELF   = "target/x86_64-unknown-none/release/vicell-kernel"
$ISO_ROOT     = "build/x86-iso-root"
$ISO_OUT      = "build/vicell-x86.iso"
$LIMINE_CFG   = "scripts/x86/limine.cfg"
$QEMU         = if (Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue) {
                    "qemu-system-x86_64"
                } elseif (Test-Path "C:\Program Files\qemu\qemu-system-x86_64.exe") {
                    "C:\Program Files\qemu\qemu-system-x86_64.exe"
                } else { $null }

# ── Locate Limine binaries (prefer apt-installed, fall back to local limine/) ──
function Find-LimineBin {
    param([string]$Name)
    # Check Windows paths first (avoids WSL CWD ambiguity with relative paths).
    $winCandidates = @(
        "limine\limine-8.7.0\bin\$Name",
        "limine\limine-8.7.0\$Name",
        "limine\$Name"
    )
    foreach ($c in $winCandidates) {
        if (Test-Path $c) {
            # Return as WSL absolute path for use in wsl bash commands.
            $abs = (Resolve-Path $c).Path
            return ("/mnt/" + $abs.Replace('\','/').Replace('D:','d').Replace('C:','c').TrimStart('/'))
        }
    }
    # Fall back to apt-installed paths inside WSL.
    $wslCandidates = @("/usr/share/limine/$Name", "/usr/lib/limine/$Name")
    foreach ($c in $wslCandidates) {
        $found = wsl bash -c "[ -f '$c' ] && echo '$c' || true"
        if ($found) { return $found }
    }
    return $null
}

# ── Step 1: Build kernel ───────────────────────────────────────────────────────
if (-not $NoBuild) {
    Write-Host "[1/4] Building x86_64 kernel..."
    cargo build --release -p vicell-kernel --target x86_64-unknown-none
    if ($LASTEXITCODE -ne 0) { Write-Error "Cargo build failed"; exit 1 }
}
if (-not (Test-Path $KERNEL_ELF)) { Write-Error "Kernel ELF not found: $KERNEL_ELF"; exit 1 }
$elfMB = [math]::Round((Get-Item $KERNEL_ELF).Length / 1MB, 2)
Write-Host "    ELF: $KERNEL_ELF ($elfMB MB)"

# ── Step 2: Assemble ISO root ──────────────────────────────────────────────────
Write-Host "[2/4] Building ISO root..."
New-Item -ItemType Directory -Force "$ISO_ROOT/EFI/BOOT" | Out-Null
New-Item -ItemType Directory -Force "$ISO_ROOT/boot/limine" | Out-Null

# Copy Limine config + kernel (both old .cfg and new Limine 8.x .conf format)
Copy-Item $LIMINE_CFG "$ISO_ROOT/boot/limine.cfg" -Force
$LIMINE_CONF = "scripts/x86/limine.conf"
if (Test-Path $LIMINE_CONF) {
    Copy-Item $LIMINE_CONF "$ISO_ROOT/boot/limine.conf" -Force
}

# Convert Windows path to WSL path for copy
$kernelWsl = ("/mnt/" + (Resolve-Path $KERNEL_ELF).Path.Replace('\','/').Replace('D:','d').Replace('C:','c').TrimStart('/'))
$isoRootWsl = ("/mnt/" + (Resolve-Path ".").Path.Replace('\','/').Replace('D:','d').Replace('C:','c').TrimStart('/') + "/$ISO_ROOT")

wsl bash -c "cp '$kernelWsl' '$isoRootWsl/boot/kernel.elf'"

# Find and copy Limine binary files
$bios_cd  = Find-LimineBin "limine-bios-cd.bin"
$bios_sys = Find-LimineBin "limine-bios.sys"
$efi_bin  = Find-LimineBin "BOOTX64.EFI"

$has_bios = $bios_cd -and $bios_sys
$has_uefi = $efi_bin -ne $null

if ($has_bios) {
    Write-Host "    Using BIOS boot: $bios_cd"
    wsl bash -c "cp '$bios_cd' '$isoRootWsl/boot/limine/limine-bios-cd.bin'"
    wsl bash -c "cp '$bios_sys' '$isoRootWsl/boot/limine/limine-bios.sys'"
}
if ($has_uefi) {
    Write-Host "    Using UEFI boot: $efi_bin"
    wsl bash -c "cp '$efi_bin' '$isoRootWsl/EFI/BOOT/BOOTX64.EFI'"
}
if (-not $has_bios -and -not $has_uefi) {
    Write-Error "No Limine binaries found. Run: wsl sudo apt-get install -y limine"
    exit 1
}

# ── Step 3: Create ISO ─────────────────────────────────────────────────────────
# Prefer genisoimage (lighter, no El Torito EFI complications), fall back to xorriso.
$iso_tool = wsl bash -c "which genisoimage 2>/dev/null || which xorriso 2>/dev/null || echo ''"
Write-Host "[3/4] Creating ISO with $iso_tool..."
$isoWsl = ("/mnt/" + (Resolve-Path ".").Path.Replace('\','/').Replace('D:','d').Replace('C:','c').TrimStart('/') + "/$ISO_OUT")
New-Item -ItemType Directory -Force "build" | Out-Null

if (-not $iso_tool) {
    Write-Error "Neither genisoimage nor xorriso found in WSL. Run: wsl sudo apt-get install -y genisoimage"
    exit 1
}

if ($has_bios) {
    $bios_rel = "boot/limine/limine-bios-cd.bin"
    if ($iso_tool -match "genisoimage") {
        wsl bash -c @"
genisoimage -R -J \
  -b $bios_rel -no-emul-boot -boot-load-size 4 -boot-info-table \
  -o '$isoWsl' '$isoRootWsl' 2>&1
"@
    } else {
        wsl bash -c @"
xorriso -as mkisofs -R -J \
  -b $bios_rel -no-emul-boot -boot-load-size 4 -boot-info-table \
  -o '$isoWsl' '$isoRootWsl' 2>&1
"@
    }
} else {
    Write-Error "BIOS boot files not found and EFI-only path removed. Check limine binaries."
    exit 1
}

if ($LASTEXITCODE -ne 0) { Write-Error "ISO creation failed"; exit 1 }
if (-not (Test-Path $ISO_OUT)) { Write-Error "ISO not created"; exit 1 }
$isoMB = [math]::Round((Get-Item $ISO_OUT).Length / 1MB, 2)
Write-Host "    ISO: $ISO_OUT ($isoMB MB)"

# ── Step 4: Boot in QEMU ──────────────────────────────────────────────────────
if ($NoQemu) { Write-Host "ISO ready. Use -NoQemu to skip QEMU."; exit 0 }
if (-not $QEMU) { Write-Error "qemu-system-x86_64 not found"; exit 1 }

Write-Host "[4/4] Booting QEMU q35..."
Write-Host "    Ctrl-a x to exit | Ctrl-a c for monitor"
Write-Host ""

Write-Host "    Boot mode: BIOS"
& $QEMU `
    -machine q35 -cpu qemu64 -m 256M `
    -cdrom $ISO_OUT -boot d `
    -serial stdio `
    -no-reboot -no-shutdown `
    -display none
