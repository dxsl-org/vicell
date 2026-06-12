//! AArch64 full-boot integration tests.
//!
//! Mirrors the RISC-V `boot.rs` suite for the ARM64 virt machine.
//!
//! Prerequisites:
//!   - `qemu-system-aarch64` on PATH (or in the Windows default install path)
//!   - Kernel built: `RUSTFLAGS="-C relocation-model=pic" cargo build --release
//!                    --target aarch64-unknown-none-softfloat -p vicell-kernel`
//!   - Disk image: `disk_arm_virt.img` at repo root (built by `format-disk-arm.ps1`
//!                 or by `tools/mkfat32.py`)
//!
//! Tests skip gracefully when any prerequisite is absent — CI behaviour is
//! identical to the RISC-V suite.

use std::path::PathBuf;
use vicell_integration_tests::{qemu_binary_aarch64, QemuRunner};

const BOOT_TIMEOUT: u64 = 45;
const CMD_TIMEOUT: u64 = 10;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root resolves")
}

fn kernel_path() -> String {
    repo_root()
        .join("target/aarch64-unknown-none-softfloat/release/vicell-kernel")
        .to_string_lossy()
        .into_owned()
}

fn disk_path() -> String {
    repo_root()
        .join("disk_arm_virt.img")
        .to_string_lossy()
        .into_owned()
}

fn prerequisites_ok() -> bool {
    let kernel_exists = PathBuf::from(kernel_path()).exists();
    let disk_exists = PathBuf::from(disk_path()).exists();
    let qemu_ok = std::process::Command::new(qemu_binary_aarch64())
        .arg("--version")
        .output()
        .is_ok();
    if !kernel_exists {
        eprintln!("SKIP aarch64: kernel not built ({})", kernel_path());
    }
    if !disk_exists {
        eprintln!("SKIP aarch64: disk_arm_virt.img missing — run .\\format-disk-arm.ps1");
    }
    if !qemu_ok {
        eprintln!("SKIP aarch64: qemu-system-aarch64 not on PATH");
    }
    kernel_exists && disk_exists && qemu_ok
}

/// The kernel must boot and emit the scheduler-initialized banner, then bring up
/// all services and reach the `ViCell >` shell prompt.
#[test]
fn aarch64_boots_to_shell_prompt() {
    if !prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_aarch64_with_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViCell >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("aarch64 shell prompt not reached: {e}\n--- output ---\n{}", qemu.dump()));
}

/// The kernel must emit its boot banner (`[ViCell] kernel boot v`) on AArch64.
///
/// This verifies the kernel's `kmain` is entered correctly after EL2→EL1 drop
/// and the PL011 UART is initialised before any subsystem setup begins.
#[test]
fn aarch64_kernel_banner() {
    if !prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_aarch64_with_disk(&kernel_path(), &disk_path());
    qemu.wait_for("[ViCell] kernel boot v", 15)
        .unwrap_or_else(|e| panic!("aarch64 kernel banner missing: {e}\n--- output ---\n{}", qemu.dump()));
}

/// The task scheduler must report it is ready before any cell is spawned.
///
/// `"Scheduler initialized"` is emitted after the frame allocator, heap, page
/// tables, and interrupt controller have all been set up successfully.
#[test]
fn aarch64_scheduler_initializes() {
    if !prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_aarch64_with_disk(&kernel_path(), &disk_path());
    qemu.wait_for("Scheduler initialized", 20)
        .unwrap_or_else(|e| panic!("aarch64 scheduler init not seen: {e}\n--- output ---\n{}", qemu.dump()));
}

/// The embedded init ELF must be spawned successfully from the kernel ramdisk.
///
/// `"Successfully spawned init"` is logged by `main.rs` when `spawn_from_mem`
/// returns `Ok` for the embedded init binary. A failure here means the EL0
/// entry path, page-table user-flag setup, or manifest parsing is broken.
#[test]
fn aarch64_init_spawns() {
    if !prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_aarch64_with_disk(&kernel_path(), &disk_path());
    qemu.wait_for("Successfully spawned init", 20)
        .unwrap_or_else(|e| panic!("aarch64 init spawn not seen: {e}\n--- output ---\n{}", qemu.dump()));
}

/// The shell must execute an interactive command.
///
/// Waits for the shell prompt, sends `echo aarch64-ok`, and asserts the
/// response appears. Proves the full path: PL011 UART RX → shell readline →
/// built-in dispatch → UART TX → serial harness.
#[test]
fn aarch64_echo_command() {
    if !prerequisites_ok() {
        return;
    }
    let mut qemu = QemuRunner::boot_aarch64_with_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViCell >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("aarch64 shell prompt not reached: {e}\n--- output ---\n{}", qemu.dump()));
    std::thread::sleep(std::time::Duration::from_millis(500));
    qemu.send_line("echo aarch64-ok");
    qemu.wait_for("aarch64-ok", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("aarch64 echo did not respond: {e}\n--- output ---\n{}", qemu.dump()));
}
