# Build a test-hooks kernel for VFS quota integration tests.
#
# Produces: target/riscv64gc-unknown-none-elf/release/vicell-kernel-test-hooks
#
# What differs from the production build:
#   - service-vfs is compiled with --features test-hooks  (2 KiB quota)
#   - app-vfs-test is compiled with --features test-hooks (includes quota test)
#   - app-init    is compiled normally — vfs-test spawn is unconditional in main.rs
#   - kernel_fs.img embeds the test-hooks cell binaries + vfs-test
#   - kernel is built with EMBEDDED_OVERRIDE pointing at the test-hooks image dir
#
# The kernel binary is separate from the production binary so it never overwrites it.
# Run prerequisites: xpack riscv GCC + libclang on PATH (same as gen_disk.ps1).

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root    = (Get-Location).Path
$tools   = "$root\tools"
$rel     = "$root\target\riscv64gc-unknown-none-elf\release"
$th_dir  = "$root\kernel\src\embedded-test-hooks"

# Toolchain env (mirrors gen_disk.ps1)
$env:CC_riscv64gc_unknown_none_elf     = "riscv-none-elf-gcc"
$env:CFLAGS_riscv64gc_unknown_none_elf = "-march=rv64gc -mabi=lp64d -mcmodel=medany -ffreestanding -DLFS_NO_INTRINSICS"
if (-not $env:LIBCLANG_PATH) {
    $vsLlvm = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\Llvm\x64\bin"
    if (Test-Path "$vsLlvm\libclang.dll") { $env:LIBCLANG_PATH = $vsLlvm }
}

# ── Step 1: build production cells (shell, config) — unchanged versions needed
# in the image so init can bootstrap the full service stack.
Write-Host "==> Building base cells (no test-hooks)..."
cargo build --release -p app-shell -p service-config 2>&1 | Select-Object -Last 3

# ── Step 2: build test-hooks variants of vfs and vfs-test.
# init is built without features; vfs-test spawn is unconditional in init/main.rs
# (silently returns NotFound in production images that do not contain vfs-test).
Write-Host "==> Building init (normal) and test-hooks cells (vfs, vfs-test)..."
cargo build --release -p app-init 2>&1 | Select-Object -Last 3
cargo build --release -p service-vfs   --features test-hooks 2>&1 | Select-Object -Last 3
cargo build --release -p app-vfs-test  --features test-hooks 2>&1 | Select-Object -Last 3

# Verify all required binaries exist.
$required = @{
    "app-init"                   = "$rel\app-init"
    "service-vfs (test-hooks)"  = "$rel\service-vfs"
    "app-vfs-test (test-hooks)" = "$rel\vfs-test"
    "app-shell"                 = "$rel\app-shell"
    "service-config"            = "$rel\service-config"
}
foreach ($kv in $required.GetEnumerator()) {
    if (-not (Test-Path $kv.Value)) {
        Write-Error "Missing: $($kv.Key) at $($kv.Value)"
    }
}

# ── Step 3: assemble kernel_fs.img from test-hooks binaries.
Write-Host "==> Building kernel_fs.img (test-hooks)..."
New-Item -ItemType Directory -Force $th_dir | Out-Null
$tmpDir = "$env:TEMP\ViCell_kfs_th"
New-Item -ItemType Directory -Force $tmpDir | Out-Null
Set-Content -Path "$tmpDir\hostname" -Value "ViCell-test" -NoNewline -Encoding ascii

python "$tools\mkfat32.py" `
    "$th_dir\kernel_fs.img" `
    "$rel\app-init"       "/bin/init" `
    "$rel\app-shell"      "/bin/shell" `
    "$rel\service-vfs"    "/bin/vfs" `
    "$rel\service-config" "/bin/config" `
    "$rel\vfs-test"       "/bin/vfs-test" `
    "$tmpDir\hostname"    "/etc/hostname"

if (-not (Test-Path "$th_dir\kernel_fs.img")) {
    Write-Error "mkfat32.py did not produce kernel_fs.img"
}
Write-Host "   kernel_fs.img: $([math]::Round((Get-Item "$th_dir\kernel_fs.img").Length / 1MB, 1)) MB"

# The kernel statically embeds init via include_bytes!(EMBEDDED_OUT_DIR/init) separate
# from kernel_fs.img.  Copy the freshly compiled app-init so the kernel's INIT_ELF
# static gets the test-hooks init (which includes the /bin/vfs-test spawn).
Copy-Item "$rel\app-init" "$th_dir\init" -Force
Write-Host "   init binary:   $([math]::Round((Get-Item "$th_dir\init").Length / 1KB, 0)) KB"

# ── Step 4: build the test-hooks kernel, pointing EMBEDDED_OVERRIDE at th_dir.
Write-Host "==> Building test-hooks kernel (riscv64, PIC)..."
$env:EMBEDDED_OVERRIDE = $th_dir
$env:RUSTFLAGS = "-C relocation-model=pic"
cargo build --release -p vicell-kernel `
    --target riscv64gc-unknown-none-elf 2>&1 | Select-Object -Last 5
Remove-Item Env:\EMBEDDED_OVERRIDE -ErrorAction SilentlyContinue
Remove-Item Env:\RUSTFLAGS          -ErrorAction SilentlyContinue

$kernel_src = "$rel\vicell-kernel"
$kernel_dst = "$rel\vicell-kernel-test-hooks"
Copy-Item $kernel_src $kernel_dst -Force
Write-Host "==> Test-hooks kernel: $kernel_dst"
Write-Host "==> Done. Run integration tests with:"
Write-Host "    cargo test --manifest-path tests/integration/Cargo.toml --target x86_64-pc-windows-msvc vfs_quota"
