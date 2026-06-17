# Generate disk images for ViCell.
#
# Two separate images are produced:
#
#   kernel\src\embedded\kernel_fs.img  (~8 MB, FAT32)
#       Embedded in the kernel binary via ramdisk.rs (include_bytes!).
#       Contains release-built cell ELFs + /hostname + /readme.
#       Served by the kernel's internal filesystem (sys_open / ReadDir).
#
#   disk_v3.img  (~455 MB, MBR — see tools/write-mbr.py and api::disk)
#       Passed to QEMU as a VirtIO block device (-drive file=disk_v3.img).
#       P1 @2048:   FAT32 interop volume (/mnt/sd)
#       P2 @526336: Cell bootstrap table read by the early loader.
#       P3 @560000: kernel heap snapshot region (Phase 29).
#       P4 @800000: littlefs /data volume (power-safe persistent store).
#       SpawnFromPath uses the P2 table to load VFS, config, shell.

$kernel_root = Get-Location
$tools_dir   = "$kernel_root\tools"
$rel_dir     = "$kernel_root\target\riscv64gc-unknown-none-elf\release"

# Toolchain for the littlefs C core inside service-vfs (littlefs2-sys):
# cross-compile with the xpack riscv gcc; bindgen needs a 64-bit libclang
# (the VS BuildTools x64 copy). LFS_NO_INTRINSICS avoids __bswapsi2/__popcountdi2
# libcalls whose prebuilt compiler-builtins objects carry a soft-float ABI tag
# and refuse to link with our lp64d objects.
$env:CC_riscv64gc_unknown_none_elf     = "riscv-none-elf-gcc"
$env:CFLAGS_riscv64gc_unknown_none_elf = "-march=rv64gc -mabi=lp64d -mcmodel=medany -ffreestanding -DLFS_NO_INTRINSICS"
if (-not $env:LIBCLANG_PATH) {
    $vsLlvm = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\Llvm\x64\bin"
    if (Test-Path "$vsLlvm\libclang.dll") { $env:LIBCLANG_PATH = $vsLlvm }
}

# 1. Build ALL cells in release mode.
# Release binaries are 10-100x smaller than debug, which matters because
# SpawnFromPath copies the full ELF into the 16MB kernel heap.
# Debug VFS=5.7MB, release VFS=3MB; Debug net=4.2MB, release net=~1MB.
Write-Host "Building release cells..."
cargo build --release `
    -p app-init -p app-shell `
    -p service-vfs -p service-config `
    -p service-input -p service-net -p service-compositor 2>&1 | Select-Object -Last 5
cargo build --release -p app-bench 2>&1 | Select-Object -Last 3   # builds bench + bench-probe
cargo build --release -p app-net-tools 2>&1 | Select-Object -Last 3
cargo build --release -p app-sys-tools 2>&1 | Select-Object -Last 3
cargo build --release -p robot-demo -p robot-dashboard 2>&1 | Select-Object -Last 3
cargo build --release -p input-test 2>&1 | Select-Object -Last 3

# DOOM — only if doomgeneric sources have been cloned
$doom_src = "cells\apps\doom\src\c\doomgeneric\doomgeneric"
if (Test-Path $doom_src) {
    Write-Host "Building DOOM cell..."
    cargo build --release -p doom --target riscv64gc-unknown-none-elf -Z build-std=core,alloc 2>&1 | Select-Object -Last 3
} else {
    Write-Host "Skipping DOOM (clone doomgeneric to $doom_src first)."
}

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
$doom_bin   = "$rel_dir\doom"              # DOOM cell (needs doomgeneric clone first)
$doom_wad   = "doom1.wad"                  # shareware WAD — place at d:/ViCell/doom1.wad
$upy_bin    = "$rel_dir\micropython"       # Phase 18: MicroPython runtime cell
$bench_bin       = "$rel_dir\bench"             # Phase 22 benchmark cell
$bench_probe_bin = "$rel_dir\bench-probe"      # bench probe/load child (VA 0x19000000)
$input_bin  = "$rel_dir\service-input"     # Phase 14: input service cell
$net_bin    = "$rel_dir\service-net"       # Phase 15: network service cell
$comp_bin      = "$rel_dir\service-compositor" # Phase 16: compositor + GPU
$robot_demo_bin = "$rel_dir\robot-demo"       # G1 sensor→actuator reference demo
$dashboard_bin = "$rel_dir\robot-dashboard"  # G1 ViUI v2 dashboard demo
$nc_bin     = "$rel_dir\nc"               # Phase A: TCP netcat tool
$curl_bin   = "$rel_dir\curl"             # Phase B: HTTP GET client
$wget_bin   = "$rel_dir\wget"             # Phase U: HTTP wget tool
$httpd_bin  = "$rel_dir\httpd"            # Phase U: HTTP server
$mqtt_bin   = "$rel_dir\mqtt"             # Phase X-5: MQTT client
$posix_shim_test_bin = "$rel_dir\posix-shim-test"  # Tier 1b POSIX shim test cell
$input_test_bin      = "$rel_dir\input-test"       # P05 bare-cell input delivery test
$ls_bin   = "$rel_dir\ls"    # M3.2 embedded debug utils
$cat_bin  = "$rel_dir\cat"
$echo_bin = "$rel_dir\echo"
$ps_bin   = "$rel_dir\ps"
$kill_bin = "$rel_dir\kill"

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

if (-not (Test-Path $doom_bin)) {
    Write-Host "Warning: DOOM binary not found — skipping DOOM in FAT32 image."
    $doom_bin = $null
}
if (-not (Test-Path $doom_wad)) {
    Write-Host "Warning: doom1.wad not found at $doom_wad — skipping WAD in FAT32 image."
    $doom_wad = $null
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
$tmpDir = "$env:TEMP\ViCell_kfs"
New-Item -ItemType Directory -Force $tmpDir | Out-Null
Set-Content -Path "$tmpDir\hostname" -Value "ViCell" -NoNewline -Encoding ascii
Set-Content -Path "$tmpDir\readme"   -Value "Welcome to ViCell!" -NoNewline -Encoding ascii
$kfs_args = @(
    "kernel\src\embedded\kernel_fs.img",
    "$rel_dir\app-init",       "/bin/init",
    "$rel_dir\app-shell",      "/bin/shell",
    "$rel_dir\service-vfs",    "/bin/vfs",
    "$rel_dir\service-config", "/bin/config",
    "$tmpDir\hostname",        "/etc/hostname",
    "$tmpDir\readme",          "/readme.txt"
)
if ($lua_bin)   { $kfs_args += @($lua_bin,  "/bin/lua") }
if ($upy_bin)   { $kfs_args += @($upy_bin,  "/bin/python") }
if ($doom_bin)  { $kfs_args += @($doom_bin, "/bin/doom") }
if ($doom_wad)  { $kfs_args += @($doom_wad, "/doom1.wad") }
if (Test-Path $ls_bin)   { $kfs_args += @($ls_bin,   "/bin/ls") }
if (Test-Path $cat_bin)  { $kfs_args += @($cat_bin,  "/bin/cat") }
if (Test-Path $echo_bin) { $kfs_args += @($echo_bin, "/bin/echo") }
if (Test-Path $ps_bin)   { $kfs_args += @($ps_bin,   "/bin/ps") }
if (Test-Path $kill_bin) { $kfs_args += @($kill_bin, "/bin/kill") }
python "$tools_dir\mkfat32.py" @kfs_args 2>&1
Remove-Item -Recurse -Force $tmpDir
$kfs_mb = [Math]::Round((Get-Item "kernel\src\embedded\kernel_fs.img").Length/1MB,1)
Write-Host "  kernel_fs.img: ${kfs_mb} MB"

# 3b. Rebuild the kernel binary (embeds the new kernel_fs.img via include_bytes!).
#     Must be done before creating disk_v3.img so the test runner picks up the latest kernel.
Write-Host "Rebuilding kernel (embedding updated kernel_fs.img)..."
$env:RUSTFLAGS = "-C relocation-model=pic"
cargo build --release -p vicell-kernel `
    --target riscv64gc-unknown-none-elf `
    -Z build-std=core,alloc 2>&1 | Select-Object -Last 3
Remove-Item Env:\RUSTFLAGS

# 3c. Create a blank disk image for VirtIO block — MBR layout (Milestone 2.5 P03).
#     P1 FAT32 @2048+524288 · P2 cell-table @526336 · P3 snapshot @560000 · P4 littlefs @800000
#     Must match tools/write-mbr.py and kernel/src/loader/disk_layout.rs.
Write-Host "Creating blank disk image (disk_v3.img, MBR, ~455 MB)..."
$disk_sectors = 931072
$diskSize = $disk_sectors * 512
$blankImg = New-Object byte[] $diskSize
[System.IO.File]::WriteAllBytes("disk_v3.img", $blankImg)
Write-Host "  Blank image created ($disk_sectors sectors)."

# 3c. Write the MBR partition table at LBA 0.
python "$tools_dir\write-mbr.py" "disk_v3.img" 2>&1
if ($LASTEXITCODE -ne 0) { throw "MBR write failed" }

# 3d. Format an empty FAT32 filesystem inside P1 (base LBA 2048).
#     65525+ data clusters at 8 sec/clus satisfy the FAT32 minimum.
Write-Host "Formatting FAT32 partition P1 (LBA 2048 + 524288 sectors)..."
python "$tools_dir\mkfat32_inplace.py" "disk_v3.img" 524288 2048 2>&1
if ($LASTEXITCODE -ne 0) { throw "FAT32 format failed — disk_v3.img may be corrupt" }

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
if ($bench_bin)       { $table_args += "/bin/bench=$bench_bin" }
if (Test-Path "$rel_dir\bench-probe") { $table_args += "/bin/bench-probe=$bench_probe_bin" }
if (Test-Path $input_bin) { $table_args += "/bin/input=$input_bin" }
if (Test-Path $net_bin)   { $table_args += "/bin/net=$net_bin" }
if (Test-Path $comp_bin)        { $table_args += "/bin/compositor=$comp_bin" }
if (Test-Path $robot_demo_bin)  { $table_args += "/bin/robot-demo=$robot_demo_bin" }
if (Test-Path $dashboard_bin)   { $table_args += "/bin/robot-dashboard=$dashboard_bin" }
if (Test-Path $nc_bin)    { $table_args += "/bin/nc=$nc_bin" }
if (Test-Path $curl_bin)  { $table_args += "/bin/curl=$curl_bin" }
if (Test-Path $wget_bin)  { $table_args += "/bin/wget=$wget_bin" }
if (Test-Path $httpd_bin) { $table_args += "/bin/httpd=$httpd_bin" }
if (Test-Path $mqtt_bin)  { $table_args += "/bin/mqtt=$mqtt_bin" }
if (Test-Path $posix_shim_test_bin) { $table_args += "/bin/posix-shim-test=$posix_shim_test_bin" }
if (Test-Path $input_test_bin)      { $table_args += "/bin/input-test=$input_test_bin" }
if (Test-Path $ls_bin)   { $table_args += "/bin/ls=$ls_bin" }
if (Test-Path $cat_bin)  { $table_args += "/bin/cat=$cat_bin" }
if (Test-Path $echo_bin) { $table_args += "/bin/echo=$echo_bin" }
if (Test-Path $ps_bin)   { $table_args += "/bin/ps=$ps_bin" }
if (Test-Path $kill_bin) { $table_args += "/bin/kill=$kill_bin" }
python "$tools_dir\write-cell-table.py" @table_args

Write-Host "Done. disk_v3.img is ready."
Write-Host ""
Write-Host "To run benchmarks: boot QEMU and run '/bin/bench' from the shell."
