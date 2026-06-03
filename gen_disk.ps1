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
$rel_dir     = "$kernel_root\target\riscv64gc-unknown-none-elf\release"

# 1. Build ALL cells in release mode.
# Release binaries are 10-100x smaller than debug, which matters because
# SpawnFromPath copies the full ELF into the 16MB kernel heap.
# Debug VFS=5.7MB, release VFS=3MB; Debug net=4.2MB, release net=~1MB.
Write-Host "Building release cells..."
cargo build --release `
    -p app-init -p app-shell `
    -p service-vfs -p service-config `
    -p service-input -p service-net -p service-compositor `
    -p micropython 2>&1 | Select-Object -Last 5
cargo build --release -p app-bench 2>&1 | Select-Object -Last 3
cargo build --release -p app-net-tools 2>&1 | Select-Object -Last 3

# 1b. Update kernel embedded cells (init, shell, vfs, config) from release builds.
# These 4 cells are embedded in kernel_fs.img via include_bytes!.
Write-Host "Updating kernel embedded cells..."
$embedded = "kernel\src\embedded"
Copy-Item "$rel_dir\app-init"       "$embedded\init"   -Force
Copy-Item "$rel_dir\app-shell"      "$embedded\shell"  -Force
Copy-Item "$rel_dir\service-vfs"    "$embedded\vfs"    -Force
Copy-Item "$rel_dir\service-config" "$embedded\config" -Force
if (Test-Path "$rel_dir\lua") { Copy-Item "$rel_dir\lua" "$embedded\lua" -Force }

# 2. Paths — all bootstrap table entries use RELEASE builds.
$init_bin   = "$rel_dir\app-init"
$shell_bin  = "$rel_dir\app-shell"
$vfs_bin    = "$rel_dir\service-vfs"
$config_bin = "$rel_dir\service-config"
$lua_bin    = "$rel_dir\lua"
$upy_bin    = "$rel_dir\micropython"       # Phase 18: MicroPython runtime cell
$bench_bin  = "$rel_dir\bench"             # Phase 22 benchmark cell
$input_bin  = "$rel_dir\service-input"     # Phase 14: input service cell
$net_bin    = "$rel_dir\service-net"       # Phase 15: network service cell
$comp_bin   = "$rel_dir\service-compositor" # Phase 16: compositor + GPU
$nc_bin     = "$rel_dir\nc"               # Phase A: TCP netcat tool
$curl_bin   = "$rel_dir\curl"             # Phase B: HTTP GET client

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

if (-not (Test-Path $upy_bin)) {
    Write-Host "Warning: MicroPython binary not found — skipping python in FAT32 image."
    $upy_bin = $null
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
if ($lua_bin)  { $kfs_args += @($lua_bin,  "/bin/lua") }
if ($upy_bin)  { $kfs_args += @($upy_bin,  "/bin/python") }
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

# 3c. Format an empty FAT16 filesystem on LBA 0-81919 (before the cell table
#     lands at LBA 82000). The VFS cell mounts this at startup as /data/.
Write-Host "Formatting FAT16 partition (LBA 0-81919)..."
python "$tools_dir\mkfat16.py" "disk_v3.img" 81920 2>&1
if ($LASTEXITCODE -ne 0) { Write-Host "Warning: mkfat16.py failed; /data writes will not persist."; }

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
if ($upy_bin)   { $table_args += "/bin/python=$upy_bin" }
if ($bench_bin) { $table_args += "/bin/bench=$bench_bin" }
if (Test-Path $input_bin) { $table_args += "/bin/input=$input_bin" }
if (Test-Path $net_bin)   { $table_args += "/bin/net=$net_bin" }
if (Test-Path $comp_bin)  { $table_args += "/bin/compositor=$comp_bin" }
if (Test-Path $nc_bin)    { $table_args += "/bin/nc=$nc_bin" }
if (Test-Path $curl_bin)  { $table_args += "/bin/curl=$curl_bin" }
python "$tools_dir\write-cell-table.py" @table_args

Write-Host "Done. disk_v3.img is ready."
Write-Host ""
Write-Host "To run benchmarks: boot QEMU and run '/bin/bench' from the shell."
