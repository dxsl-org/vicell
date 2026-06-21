//! VirtIO PCI integration tests for x86_64 QEMU q35.
//!
//! Exercises the VirtIO PCI discovery path introduced in Phase 03
//! (`kernel/src/task/drivers/virtio_pci.rs`). QEMU q35 exposes VirtIO
//! devices over PCIe (vendor 0x1AF4) rather than the fixed MMIO bases
//! used on ARM64/RISC-V.
//!
//! Prerequisites:
//!   - `qemu-system-x86_64` on PATH (or at the Windows default install path)
//!   - ISO built at `build/vicell-x86.iso`
//!
//! Tests skip gracefully when any prerequisite is absent.

use std::path::PathBuf;
use vicell_integration_tests::{qemu_binary_x86, QemuRunner};

const BOOT_TIMEOUT: u64 = 45;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root resolves")
}

fn iso_path() -> String {
    repo_root()
        .join("build/vicell-x86.iso")
        .to_string_lossy()
        .into_owned()
}

fn prerequisites_ok() -> bool {
    let iso_exists = PathBuf::from(iso_path()).exists();
    let qemu_ok = std::process::Command::new(qemu_binary_x86())
        .arg("--version")
        .output()
        .is_ok();
    if !iso_exists {
        eprintln!("SKIP virtio-x86: ISO not built ({})", iso_path());
    }
    if !qemu_ok {
        eprintln!("SKIP virtio-x86: qemu-system-x86_64 not found");
    }
    iso_exists && qemu_ok
}

/// Create a small raw disk image filled with a known pattern for VirtIO BLK read testing.
fn make_virtio_disk() -> tempfile::NamedTempFile {
    let disk = tempfile::Builder::new()
        .suffix(".virtio.img")
        .tempfile()
        .expect("tempfile for virtio disk");
    // 4 MiB zeroed disk — enough for a partition table the kernel could scan.
    let data = vec![0u8; 4 * 1024 * 1024];
    std::fs::write(disk.path(), &data).expect("write virtio disk");
    disk
}

/// The kernel must log that VirtIO PCI block device is initialised when
/// a `virtio-blk-pci` device is attached under QEMU q35.
///
/// Proves: ECAM scan finds vendor 0x1AF4, BAR MMIO is mapped, VirtIOBlk
/// `new()` succeeds, and the device is registered in the kernel block store.
#[test]
fn x86_virtio_blk_initialises() {
    if !prerequisites_ok() {
        return;
    }
    let disk = make_virtio_disk();
    let disk_path = disk.path().to_string_lossy().into_owned();
    let qemu = QemuRunner::boot_x86_virtio_blk(&iso_path(), &disk_path);
    // The kernel logs this after VirtIOBlk::new() succeeds in virtio_pci::init().
    qemu.wait_for("VirtIO Block: initialized", BOOT_TIMEOUT)
        .unwrap_or_else(|e| {
            panic!(
                "x86_64 VirtIO block init not seen: {e}\n--- output ---\n{}",
                qemu.dump()
            )
        });
}

/// The VirtIO PCI probe must still succeed (or gracefully skip) even when
/// the kernel boots without a virtio-blk-pci device — ECAM scan should
/// complete, find no vendor 0x1AF4, and continue to shell.
///
/// Regression guard: P03 must not regress the baseline 5-test boot suite.
#[test]
fn x86_virtio_pci_no_device_no_hang() {
    if !prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_x86_bios(&iso_path());
    // Shell must still appear even without any VirtIO device attached.
    qemu.wait_for("ViCell >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| {
            panic!(
                "x86_64 boot hung after VirtIO PCI probe with no device: {e}\n--- output ---\n{}",
                qemu.dump()
            )
        });
}
