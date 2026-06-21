#!/usr/bin/env pwsh
# Quick UEFI boot test — captures serial output for 15s then kills QEMU
$ovmf   = "D:\ViCell\build\ovmf-x86.fd"
$iso    = "D:\ViCell\build\vicell-x86.iso"
$serial = "D:\ViCell\build\serial-x86.log"

# Clear previous log
if (Test-Path $serial) { Remove-Item $serial }

Write-Host "Booting ViCell x86_64 (UEFI) — serial log: $serial"
Write-Host "Press Ctrl+C to stop early."

$p = Start-Process -FilePath "C:\Program Files\qemu\qemu-system-x86_64.exe" -ArgumentList @(
    "-machine",  "q35",
    "-cpu",      "qemu64",
    "-m",        "256M",
    "-drive",    "if=pflash,format=raw,readonly=on,file=$ovmf",
    "-cdrom",    $iso,
    "-boot",     "d",
    "-serial",   "file:$serial",
    "-display",  "none",
    "-no-reboot"
) -PassThru -NoNewWindow

# Wait up to 40 seconds for boot (includes Limine menu + kernel boot)
$timeout = 40
for ($i = 0; $i -lt $timeout; $i++) {
    Start-Sleep 1
    if (Test-Path $serial) {
        $content = Get-Content $serial -Raw -ErrorAction SilentlyContinue
        if ($content -match "Scheduler initialized|PANIC|page fault|triple fault") {
            Write-Host "`n=== Early exit at ${i}s ==="
            break
        }
    }
}

# Kill QEMU
if (!$p.HasExited) { $p.Kill() }

# Show full log
if (Test-Path $serial) {
    Write-Host "`n=== Full serial log ==="
    Get-Content $serial
} else {
    Write-Host "No serial output captured."
}
