//! End-to-end integration test for the robot-demo G1 graduation criterion.
//!
//! Boots the AArch64 ARM virt image with a disk, lets `init` spawn `/bin/robot-demo`,
//! and asserts the full sensor→compute→actuator pipeline ran to completion.
//!
//! AArch64 is required: the real GPIO/I2C path (PL061 + BitBangI2c) only exists
//! on the ARM virt machine. RISC-V falls through to `simulate_loop` before the
//! bus is touched, so the meaningful E2E proof is ARM64.
//!
//! Prerequisites:
//!   - `qemu-system-aarch64` on PATH
//!   - Kernel: `target/aarch64-unknown-none-softfloat/release/vicell-kernel`
//!   - Disk: `disk_arm_virt.img` at repo root (built by `format-disk-arm.ps1`)
//!
//! Tests skip gracefully (exit 0) when any prerequisite is absent — same policy
//! as `periph-i2c-spi.rs`. CI behaviour: skip = green, not failure.

use std::path::PathBuf;
use vicell_integration_tests::{qemu_binary_aarch64, QemuRunner};

/// Allow the full bring-up sequence: supervised services + best-effort demo cells
/// spawn last. Matches `periph-i2c-spi.rs`.
const BOOT_TIMEOUT: u64 = 60;

// ── Panic markers (from actual panic handlers) ────────────────────────────────
// kernel/src/main.rs:483  — true kernel panic
const KERNEL_PANIC_MARKER: &str = "[KERNEL PANIC]";
// libs/ostd/src/startup.rs:67 — cell/app panic via no_std panic handler
const CELL_PANIC_MARKER: &str = "PANIC: Application crashed!";

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

/// Skip when any prerequisite is absent.
fn prerequisites_ok() -> bool {
    let kernel_exists = PathBuf::from(kernel_path()).exists();
    let disk_exists = PathBuf::from(disk_path()).exists();
    let qemu_ok = std::process::Command::new(qemu_binary_aarch64())
        .arg("--version")
        .output()
        .is_ok();
    if !kernel_exists {
        eprintln!("SKIP robot-demo-e2e: kernel not built ({})", kernel_path());
    }
    if !disk_exists {
        eprintln!(
            "SKIP robot-demo-e2e: disk_arm_virt.img missing — run .\\format-disk-arm.ps1"
        );
    }
    if !qemu_ok {
        eprintln!("SKIP robot-demo-e2e: qemu-system-aarch64 not on PATH");
    }
    kernel_exists && disk_exists && qemu_ok
}

/// G1 graduation criterion 8 — primary CI gate.
///
/// Asserts the full sensor→compute→actuator→completion pipeline:
/// 1. Banner appears — demo started.
/// 2. `T=…C H=…%` appears — sensor read happened (real or synthetic both print this).
/// 3. `relay=on` or `relay=off` appears — actuator was driven from the temperature reading.
/// 4. `done (5 cycles)` appears — all 5 iterations completed without hanging.
/// 5. No kernel or cell panic marker in output.
///
/// The `[sim]` tag on every reading is expected on QEMU (no physical SHT3x slave answers
/// the bus); the assertions accept synthetic data as proof of the pipeline.
#[test]
fn aarch64_robot_demo_e2e() {
    if !prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_aarch64_with_disk(&kernel_path(), &disk_path());

    // Anchor on the completion line first — proves the full loop ran.
    qemu.wait_for("[robot-demo] done (5 cycles)", BOOT_TIMEOUT)
        .unwrap_or_else(|e| {
            panic!(
                "robot-demo did not complete all 5 cycles: {e}\n--- output ---\n{}",
                qemu.dump()
            )
        });

    let out = qemu.dump();

    assert!(
        out.contains("[robot-demo] ViCell reference robot demo"),
        "robot-demo banner missing\n--- output ---\n{out}"
    );
    assert!(
        out.contains("T=") && out.contains("H="),
        "no sensor reading (T=/H= not found) — sensor path did not execute\n--- output ---\n{out}"
    );
    assert!(
        out.contains("relay=on") || out.contains("relay=off"),
        "actuator not driven (relay= not found) — compute/actuator path did not execute\n--- output ---\n{out}"
    );
    assert!(
        !out.contains(KERNEL_PANIC_MARKER),
        "kernel panic detected\n--- output ---\n{out}"
    );
    assert!(
        !out.contains(CELL_PANIC_MARKER),
        "cell/app panic detected\n--- output ---\n{out}"
    );
}

/// Deep MQTT publish check — opt-in, NOT the G1 CI gate.
///
/// # Why `#[ignore]`
/// The guest dials `10.0.2.2:1883` (QEMU SLIRP gateway). `spawn_mqtt_broker` binds
/// a host loopback port, but SLIRP maps host→guest direction — not guest→host. To
/// intercept the guest's outbound MQTT CONNECT, you need a broker listening on the
/// loopback at port 1883 *before* QEMU starts (or use `-netdev user,hostfwd=tcp::1883-:1883`
/// pointing to a host listener). This setup is fiddly and flake-prone in CI, so this
/// test is a local/manual deep-check only.
///
/// To run manually:
///   `cargo test --manifest-path tests/integration/Cargo.toml \
///       --target x86_64-pc-windows-msvc --test robot-demo-e2e -- --ignored`
#[ignore]
#[test]
fn aarch64_robot_demo_mqtt_publish() {
    if !prerequisites_ok() {
        return;
    }
    // Boot with net hostfwd so the guest SLIRP 10.0.2.2:1883 can be intercepted.
    // Requires a host MQTT listener at 127.0.0.1:1883 before QEMU starts.
    let qemu = QemuRunner::boot_aarch64_with_disk(&kernel_path(), &disk_path());
    qemu.wait_for("[robot-demo] MQTT telemetry published", BOOT_TIMEOUT)
        .unwrap_or_else(|e| {
            panic!(
                "robot-demo did not publish MQTT telemetry: {e}\n--- output ---\n{}",
                qemu.dump()
            )
        });
    let out = qemu.dump();
    assert!(
        out.contains("MQTT telemetry published"),
        "MQTT publish confirmation missing\n--- output ---\n{out}"
    );
}
