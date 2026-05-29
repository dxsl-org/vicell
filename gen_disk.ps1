# Generate Disk Image — FAT32 primary filesystem + cell bootstrap table.
#
# Layout of disk_v3.img:
#   LBA       0 – 81 999 : FAT32 filesystem (~42 MB), served by the VFS Cell.
#   LBA  82 000 +        : Cell bootstrap table (header + entries + raw ELFs),
#                          read by the kernel early loader before VFS is up.
#
# The bootstrap table lets init spawn VFS, config, and shell via SpawnFromPath
# without embedding those ELFs in the init binary.

$kernel_root = Get-Location
$tools_dir   = "$kernel_root\tools"
$target_dir  = "$kernel_root\target\riscv64gc-unknown-none-elf\debug"
$rel_dir     = "$kernel_root\target\riscv64gc-unknown-none-elf\release"

# 1. Build all cells
Write-Host "Building cells..."
cargo build -p app-init
cargo build -p app-shell
cargo build -p service-vfs
cargo build -p service-config

# 2. Paths
$init_bin   = "$target_dir\app-init"
$shell_bin  = "$target_dir\app-shell"
$vfs_bin    = "$target_dir\service-vfs"
$config_bin = "$target_dir\service-config"
$lua_bin    = "$rel_dir\lua"
$mpy_bin    = "$rel_dir\micropython"

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

# 3. Generate FAT32 image
Write-Host "Generating FAT32 filesystem (disk_v3.img)..."
$fat_args = @("disk_v3.img", $init_bin, "init", $shell_bin, "shell",
              $vfs_bin, "vfs", $config_bin, "config")
if ($lua_bin) { $fat_args += @($lua_bin, "lua") }
if (Test-Path $mpy_bin) { $fat_args += @($mpy_bin, "micropython") }
python "$tools_dir\mkfat32.py" @fat_args

# 4. Append cell bootstrap table (for kernel early loader)
Write-Host "Appending cell bootstrap table..."
$table_args = @(
    "disk_v3.img",
    "/bin/vfs=$vfs_bin",
    "/bin/config=$config_bin",
    "/bin/shell=$shell_bin"
)
if ($lua_bin) { $table_args += "/bin/lua=$lua_bin" }
python "$tools_dir\write-cell-table.py" @table_args

Write-Host "Done. disk_v3.img is ready."
