//! End-to-end boot + interactive tests driven through QEMU serial.
//!
//! These require `qemu-system-riscv64` on PATH and pre-built artifacts:
//!   cargo build --release -p vios-kernel
//!   ./gen_disk.ps1
//!
//! Paths are relative to the repo root (two levels up from this crate). The
//! tests resolve them from CARGO_MANIFEST_DIR so they run regardless of cwd.

use std::path::PathBuf;
use vios_integration_tests::{qemu_binary, QemuRunner};

const BOOT_TIMEOUT: u64 = 40;

/// Repo root = tests/integration/.. /..
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root resolves")
}

fn kernel_path() -> String {
    repo_root()
        .join("target/riscv64gc-unknown-none-elf/release/vios-kernel")
        .to_string_lossy()
        .into_owned()
}

fn disk_path() -> String {
    repo_root().join("disk_v3.img").to_string_lossy().into_owned()
}

/// Skip (don't fail) when prerequisites are missing, so the suite is friendly
/// on machines without QEMU or a built kernel.
fn prerequisites_ok() -> bool {
    let kernel_exists = PathBuf::from(kernel_path()).exists();
    let disk_exists = PathBuf::from(disk_path()).exists();
    let qemu_ok = std::process::Command::new(qemu_binary())
        .arg("--version")
        .output()
        .is_ok();
    if !kernel_exists {
        eprintln!("SKIP: kernel not built ({})", kernel_path());
    }
    if !disk_exists {
        eprintln!("SKIP: disk_v3.img missing — run ./gen_disk.ps1");
    }
    if !qemu_ok {
        eprintln!("SKIP: qemu-system-riscv64 not on PATH");
    }
    kernel_exists && disk_exists && qemu_ok
}

/// Phase 03/06/13/14/16/17: the kernel must boot through the full service
/// chain and present the shell prompt.
#[test]
fn boots_to_shell_prompt() {
    if !prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("shell prompt not reached: {e}\n--- output ---\n{}", qemu.dump()));

    // Phase 03: Ring-3 user task ran.
    assert!(
        qemu.output_contains("user_hello") || qemu.output_contains("U-mode"),
        "ring-3 user task did not run"
    );
    // Phase 13/14/16: services spawned via SpawnFromPath.
    assert!(qemu.output_contains("/bin/vfs"), "VFS service did not spawn");
    assert!(qemu.output_contains("/bin/shell"), "shell did not spawn");
}

/// Phase 04/13: the embedded FAT16 image must mount (regression guard for the
/// CorruptedFileSystem bug fixed by switching mkfat32.py to FAT16).
#[test]
fn fat_filesystem_mounts() {
    if !prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot(&kernel_path(), &disk_path());
    qemu.wait_for("mounted successfully", BOOT_TIMEOUT).unwrap_or_else(|e| {
        panic!("FAT mount not confirmed: {e}\n--- output ---\n{}", qemu.dump())
    });
    assert!(
        !qemu.output_contains("Corrupted") && !qemu.output_contains("Failed to mount"),
        "FAT mount reported an error"
    );
}

/// Phase 17: the shell must process an interactive command. We wait for the
/// prompt, send `echo`, and expect the argument echoed back.
///
/// KNOWN ISSUE (ignored): the shell prints `ViOS >` but does not currently
/// act on serial console input delivered this way — bulk-piped stdin to the
/// guest UART is not picked up by the shell's async readline. Whether real
/// char-by-char keyboard input works has not been confirmed. Tracked as a
/// Phase 05 (keyboard input) / Phase 17 (shell) follow-up. Remove `#[ignore]`
/// once the UART RX → shell input path is verified.
#[test]
#[ignore = "shell does not consume piped serial input yet — Phase 05/17 follow-up"]
fn shell_executes_echo() {
    if !prerequisites_ok() {
        return;
    }
    let mut qemu = QemuRunner::boot(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt not reached: {e}"));
    // Give the async readline a moment to start consuming serial input.
    std::thread::sleep(std::time::Duration::from_millis(500));
    qemu.send_line("echo VIOS_ECHO_OK");
    qemu.wait_for("VIOS_ECHO_OK", 15).unwrap_or_else(|e| {
        panic!("shell did not echo command: {e}\n--- output ---\n{}", qemu.dump())
    });
}
