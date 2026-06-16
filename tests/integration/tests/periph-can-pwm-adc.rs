//! Peripheral integration tests: PWM bit-bang, ADC simulation, CAN loopback.
//!
//! These tests boot the AArch64 ARM virt image and assert that the new
//! peripheral demo cells print their expected probe strings within the boot timeout.
//!
//! Prerequisites:
//!   - `qemu-system-aarch64` on PATH
//!   - Kernel: `target/aarch64-unknown-none-softfloat/release/vicell-kernel`
//!   - Disk: `disk_arm_virt.img` at repo root (built by `format-disk-arm.ps1`)
//!
//! Tests skip gracefully when any prerequisite is absent — same pattern as
//! `periph-i2c-spi.rs`. CI behaviour: skip = exit 0 (green), not failure.

use std::path::PathBuf;
use vicell_integration_tests::{qemu_binary_aarch64, QemuRunner};

const BOOT_TIMEOUT: u64 = 90; // Extra time for 3 additional demo cells.

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
        eprintln!("SKIP periph-can-pwm-adc: kernel not built ({})", kernel_path());
    }
    if !disk_exists {
        eprintln!(
            "SKIP periph-can-pwm-adc: disk_arm_virt.img missing — run .\\format-disk-arm.ps1"
        );
    }
    if !qemu_ok {
        eprintln!("SKIP periph-can-pwm-adc: qemu-system-aarch64 not on PATH");
    }
    kernel_exists && disk_exists && qemu_ok
}

/// PWM bit-bang demo: channel 6/pin 6 on PL061, 50 Hz.
/// The demo sweeps duty from 0‰ to 1000‰ and prints after each step.
/// Test passes on the first duty line — QEMU may kill the cell before the full sweep.
#[test]
fn aarch64_pwm_demo() {
    if !prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_aarch64_with_disk(&kernel_path(), &disk_path());
    qemu.wait_for("[pwm-demo] duty=", BOOT_TIMEOUT)
        .unwrap_or_else(|e| {
            panic!(
                "pwm-demo probe not seen: {e}\n--- output ---\n{}",
                qemu.dump()
            )
        });
}

/// ADC simulation demo: 3 channels, 5 sample iterations, no MMIO.
/// Asserts the step-0 output line which is always produced first.
#[test]
fn aarch64_adc_demo() {
    if !prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_aarch64_with_disk(&kernel_path(), &disk_path());
    qemu.wait_for("[adc-demo] ch0=", BOOT_TIMEOUT)
        .unwrap_or_else(|e| {
            panic!(
                "adc-demo probe not seen: {e}\n--- output ---\n{}",
                qemu.dump()
            )
        });
}

/// CAN loopback demo: 5 frames TX + RX, 500 kbps (simulated), no MMIO.
/// Asserts the RX confirmation line from the first frame received.
#[test]
fn aarch64_can_demo() {
    if !prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_aarch64_with_disk(&kernel_path(), &disk_path());
    qemu.wait_for("[can-demo] RX id=", BOOT_TIMEOUT)
        .unwrap_or_else(|e| {
            panic!(
                "can-demo RX probe not seen: {e}\n--- output ---\n{}",
                qemu.dump()
            )
        });
}
