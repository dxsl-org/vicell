# Generate disk images for ViOS.
#
# Two separate images are produced:
#
#   kernel\src\embedded\kernel_fs.img  (~8 MB, FAT32)
#       Embedded in the kernel binary via ramdisk.rs (include_bytes!).
#       Contains release-built cell ELFs + /hostname + /readme.
#       Served by the kernel's internal filesystem (sys_open / ReadDir).
#
#   disk_v3.img  (~40 MB, blank FAT32 area + cell bootstrap table)
#       Passed to QEMU as a VirtIO block device (-drive file=disk_v3.img).
#       LBA 82000+: Cell bootstrap table read by the early loader.
#       SpawnFromPath uses this table to load VFS, config, shell.

$kernel_root = Get-Location
$tools_dir   = "$kernel_root\tools"
$target_dir  = "$kernel_root\target\riscv64gc-unknown-none-elf\debug"
$rel_dir     = "$kernel_root\target\riscv64gc-unknown-none-elf\release"

# 1. Build release cells (needed for kernel_fs.img and bootstrap table)
Write-Host "Building release cells..."
cargo build --release -p app-init -p app-shell -p service-vfs -p service-config 2>&1 | Select-Object -Last 3

# Build debug cells for bootstrap table (faster; SpawnFromPath loads these at boot)
Write-Host "Building debug cells for bootstrap table..."
cargo build -p service-vfs -p service-config -p app-shell -p service-input -p service-net
cargo build -p app-bench        # Phase 22: benchmarking cell

# 2. Paths
$init_bin   = "$target_dir\app-init"
$shell_bin  = "$target_dir\app-shell"
$vfs_bin    = "$target_dir\service-vfs"
$config_bin = "$target_dir\service-config"
$lua_bin    = "$rel_dir\lua"
$bench_bin  = "$target_dir\bench"       # Phase 22 benchmark cell
$input_bin  = "$target_dir\service-input"  # Phase 14: input service cell
$net_bin    = "$target_dir\service-net"    # Phase 15: network service cell

foreach ($pair in @(
    @{ Path = $init_bin;   Name = "app-init" },
    @{ Path = $shell_bin;  Name = "app-shell" },
    @{ Path = $vfs_bin;    Name = "service-vfs" },
    @{ Path = $config_bin; Name = "service-config" }
)) {
    if (-not (Test-Path $pair.Path)) {
        Write-Host "Error: $($pair.Name) not found at $($pair.Path)"
        exit 1
    }
}

if (-not (Test-Path $lua_bin)) {
    Write-Host "Warning: Lua binary not found — skipping Lua in FAT32 image."
    $lua_bin = $null
}

if (-not (Test-Path $bench_bin)) {
    Write-Host "Warning: bench binary not found — run 'cargo build -p app-bench' first."
    $bench_bin = $null
}

# 3a. Generate kernel_fs.img (small embedded FAT32, ~8 MB, with release cells).
#     This image is embedded in the kernel binary via ramdisk.rs.
Write-Host "Generating kernel_fs.img (embedded FAT32, release cells)..."
$tmpDir = "$env:TEMP\vios_kfs"
New-Item -ItemType Directory -Force $tmpDir | Out-Null
Set-Content -Path "$tmpDir\hostname" -Value "vios" -NoNewline -Encoding ascii
Set-Content -Path "$tmpDir\readme"   -Value "Welcome to ViOS!" -NoNewline -Encoding ascii
$kfs_args = @(
    "kernel\src\embedded\kernel_fs.img",
    "$rel_dir\app-init",       "/bin/init",
    "$rel_dir\app-shell",      "/bin/shell",
    "$rel_dir\service-vfs",    "/bin/vfs",
    "$rel_dir\service-config", "/bin/config",
    "$tmpDir\hostname",        "/etc/hostname",
    "$tmpDir\readme",          "/readme.txt"
)
if ($lua_bin) { $kfs_args += @($lua_bin, "/bin/lua") }
python "$tools_dir\mkfat32.py" @kfs_args 2>&1
Remove-Item -Recurse -Force $tmpDir
$kfs_mb = [Math]::Round((Get-Item "kernel\src\embedded\kernel_fs.img").Length/1MB,1)
Write-Host "  kernel_fs.img: ${kfs_mb} MB"

# 3b. Create a blank disk image (40MB = 81920 sectors) for VirtIO block.
Write-Host "Creating blank disk image (disk_v3.img)..."
$diskSize = 81920 * 512     # 40 MB — matches CELL_TABLE_BASE_LBA = 82000
$blankImg = New-Object byte[] $diskSize
[System.IO.File]::WriteAllBytes("disk_v3.img", $blankImg)
Write-Host "  Blank 40 MB image created."

# 4. Append cell bootstrap table (for kernel early loader).
# Only include the cells that the kernel early loader needs: VFS, config, shell.
# Optionally include lua and bench when built.
Write-Host "Appending cell bootstrap table..."
$table_args = @(
    "disk_v3.img",
    "/bin/vfs=$vfs_bin",
    "/bin/config=$config_bin",
    "/bin/shell=$shell_bin"
)
if ($lua_bin)   { $table_args += "/bin/lua=$lua_bin" }
if ($bench_bin) { $table_args += "/bin/bench=$bench_bin" }
if (Test-Path $input_bin) { $table_args += "/bin/input=$input_bin" }
if (Test-Path $net_bin)   { $table_args += "/bin/net=$net_bin" }
python "$tools_dir\write-cell-table.py" @table_args

Write-Host "Done. disk_v3.img is ready."
Write-Host ""
Write-Host "To run benchmarks: boot QEMU and run '/bin/bench' from the shell."
