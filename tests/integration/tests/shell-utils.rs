//! Shell utility integration test (Phase E — Shell M3.1).
//!
//! Boots a shell-test kernel (compiled with `app-shell --features shell_test`)
//! and waits for the `[shell-test] ALL TESTS PASSED` marker.  The marker is
//! printed by `cells/tools/shell/src/shell_test.rs` after all 9 scenarios pass.
//!
//! Prerequisites:
//!   bash scripts/build-shell-test-ci.sh
//!   → produces target/riscv64gc-unknown-none-elf/release/vicell-kernel-shell-test
//!
//! Run via:
//!   cargo test --manifest-path tests/integration/Cargo.toml --test shell-utils

use std::path::PathBuf;
use vicell_integration_tests::{qemu_binary, QemuRunner};

/// Timeout for the whole test suite to finish inside the guest.
const SUITE_TIMEOUT: u64 = 120;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root resolves")
}

fn shell_test_kernel() -> String {
    repo_root()
        .join("target/riscv64gc-unknown-none-elf/release/vicell-kernel-shell-test")
        .to_string_lossy()
        .into_owned()
}

/// Skip gracefully when prerequisites are missing (no QEMU or no built kernel).
fn prerequisites_ok() -> bool {
    let kernel_path = shell_test_kernel();
    let kernel_exists = PathBuf::from(&kernel_path).exists();
    let qemu_ok = std::process::Command::new(qemu_binary())
        .arg("--version")
        .output()
        .is_ok();
    if !kernel_exists {
        eprintln!("SKIP: shell-test kernel not built ({})", kernel_path);
        eprintln!("      Run: bash scripts/build-shell-test-ci.sh");
    }
    if !qemu_ok {
        eprintln!("SKIP: qemu-system-riscv64 not on PATH");
    }
    kernel_exists && qemu_ok
}

/// Phase E: boot the shell-test kernel and wait for all scenario tests to pass.
///
/// The shell-test cell runs `shell_test::run()` on startup, exercises all
/// Phase 1–3 shell features (stderr redirect, tee, sed, fg/bg, pipes), and
/// prints `[shell-test] ALL TESTS PASSED` when everything passes.
#[test]
fn shell_utils_all_scenarios_pass() {
    if !prerequisites_ok() {
        return;
    }
    let kernel = shell_test_kernel();
    // boot_rv64: minimal config (no disk, no VirtIO peripherals).
    // The shell-test kernel embeds init + vfs + shell-test in its kernel_fs.img.
    let qemu = QemuRunner::boot_rv64(&kernel);

    qemu.wait_for("[shell-test] ALL TESTS PASSED", SUITE_TIMEOUT)
        .unwrap_or_else(|e| {
            panic!(
                "shell-test suite did not pass within {}s: {}\n--- serial output ---\n{}",
                SUITE_TIMEOUT,
                e,
                qemu.dump()
            )
        });
}
