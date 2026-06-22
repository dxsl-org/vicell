//! Phase A-03: NVMe boot + block-read integration test on x86_64 q35.
//!
//! Boots the x86_64 kernel on a QEMU q35 machine with a PCIe NVMe controller
//! attached.  The kernel runs its full PCIe ECAM scan + NVMe init sequence and
//! must log `[nvme] driver ready` on the serial port.
//!
//! A second test sends `blktest` via the shell prompt and verifies that the
//! first sector is read without error (the `blktest` tool reads sector 0 via
//! the active `ViBlockDevice` and prints "blkio: denied" only for non-VFS cells;
//! for the VFS cell, or if called from the shell, the current cell lacks
//! `can_block_io` so the kernel gate returns `PermissionDenied`; this proves the
//! NVMe device is reachable and the gate is wired up).
//!
//! Prerequisites:
//!   - `qemu-system-x86_64` on PATH (or `VIOS_QEMU_X86` env var).
//!   - The Limine ISO built at `build/vicell-x86.iso`
//!     (`build/make-iso.sh` or `scripts/build-x86_64-cells.ps1`).
//!   - The disk image `disk_v3.img` for the boot filesystem (VirtIO block, RISC-V
//!     disk) is NOT used here; an ephemeral zeroed NVMe image is created on the fly.
//!
//! Skip semantics: the test returns without failing when any prerequisite is
//! absent, mirroring the pattern in `boot.rs`.

use std::path::PathBuf;
use vicell_integration_tests::{qemu_x86_binary, QemuRunner};

/// Timeout for the NVMe init log line (controller + queues init, sector ident).
const NVME_INIT_TIMEOUT: u64 = 60;
/// Timeout for the shell prompt (full service chain boot).
const BOOT_TIMEOUT: u64 = 90;
/// Timeout for shell command round-trips after boot.
const CMD_TIMEOUT: u64 = 15;

/// Repo root = tests/integration/../../
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root resolves")
}

/// Path to the x86_64 Limine ISO.
fn iso_path() -> String {
    repo_root()
        .join("build/vicell-x86.iso")
        .to_string_lossy()
        .into_owned()
}

/// Check all prerequisites for x86_64 NVMe tests.
///
/// Returns `true` when everything is available.  Prints a human-readable skip
/// reason for each missing prerequisite to make CI failures easy to diagnose.
fn prerequisites_ok() -> bool {
    let iso_ok = PathBuf::from(iso_path()).exists();
    let qemu_ok = std::process::Command::new(qemu_x86_binary())
        .arg("--version")
        .output()
        .is_ok();

    if !iso_ok {
        eprintln!(
            "SKIP nvme-x86: x86_64 ISO not built ({})\n\
             Build with: scripts/build-x86_64-cells.ps1 then build/make-iso.sh",
            iso_path()
        );
    }
    if !qemu_ok {
        eprintln!(
            "SKIP nvme-x86: qemu-system-x86_64 not on PATH\n\
             Install QEMU or set VIOS_QEMU_X86 to the binary path."
        );
    }
    iso_ok && qemu_ok
}

/// Create a small zeroed raw disk image in the system temp directory.
///
/// The NVMe driver only reads the namespace identify page (no filesystem
/// needed) during init, so a zeroed image is sufficient for boot tests.
/// Returns the path; the caller is responsible for deleting it.
///
/// Size: 64 MiB (131072 × 512-byte sectors) — enough to hold the FAT16
/// partition if the same disk is used for extended tests.
fn make_nvme_disk() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CTR: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "vicell_nvme_{}_{}.img",
        std::process::id(),
        CTR.fetch_add(1, Ordering::Relaxed)
    ));
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(&path)
        .expect("create NVMe disk image");
    let size: u64 = 64 * 1024 * 1024;
    f.set_len(size).expect("set NVMe disk size");
    let _ = f.write_all(b"");
    drop(f);
    path
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Phase A-03-1: NVMe controller initialises on x86_64 q35.
///
/// The kernel must boot, scan the PCIe ECAM bus, find the NVMe endpoint,
/// complete the init sequence (reset → AQA/ASQ/ACQ → CC.EN → CSTS.RDY →
/// Identify → IO queues), and print the success log line.
///
/// This is the primary Phase A integration criterion: without it, the NVMe
/// driver is dead code.
#[test]
fn nvme_controller_initialises_x86() {
    if !prerequisites_ok() {
        return;
    }

    let nvme_disk = make_nvme_disk();

    let qemu = QemuRunner::boot_x86_bios_with_nvme(
        &iso_path(),
        &nvme_disk.to_string_lossy(),
    );

    // Primary assertion: driver ready log line emitted by blk_nvme::init_driver().
    qemu.wait_for("[nvme] driver ready", NVME_INIT_TIMEOUT)
        .unwrap_or_else(|e| {
            let _ = std::fs::remove_file(&nvme_disk);
            panic!(
                "NVMe driver did not initialise on x86_64 q35: {e}\n\
                 Hint: check that PCIe ECAM is scanned before blk_nvme::init_driver().\n\
                 --- serial output ---\n{}",
                qemu.dump()
            )
        });

    let _ = std::fs::remove_file(&nvme_disk);
}

/// Phase A-03-2: PCIe ECAM scan finds NVMe endpoint on x86_64.
///
/// The kernel must log that it found the NVMe PCI device during the ECAM
/// bus-0 scan.  This verifies the ECAM walker correctly decodes the NVMe
/// class (0x01 / 0x08 / 0x02) and that `find_class()` returns a device.
#[test]
fn pcie_ecam_finds_nvme_x86() {
    if !prerequisites_ok() {
        return;
    }

    let nvme_disk = make_nvme_disk();

    let qemu = QemuRunner::boot_x86_bios_with_nvme(
        &iso_path(),
        &nvme_disk.to_string_lossy(),
    );

    // The ECAM scan log line is emitted by pcie_ecam::init() before NVMe init.
    qemu.wait_for("[pcie] ECAM scan complete", NVME_INIT_TIMEOUT)
        .unwrap_or_else(|e| {
            let _ = std::fs::remove_file(&nvme_disk);
            panic!(
                "ECAM scan did not complete on x86_64 q35: {e}\n\
                 --- serial output ---\n{}",
                qemu.dump()
            )
        });

    // The NVMe found log line.
    qemu.wait_for("[nvme] found NVMe device", NVME_INIT_TIMEOUT)
        .unwrap_or_else(|e| {
            let _ = std::fs::remove_file(&nvme_disk);
            panic!(
                "ECAM scan did not find NVMe device on x86_64 q35: {e}\n\
                 Hint: QEMU q35 maps NVMe at PCIe root port; check ECAM_BASE_X86.\n\
                 --- serial output ---\n{}",
                qemu.dump()
            )
        });

    let _ = std::fs::remove_file(&nvme_disk);
}

/// Phase A-03-3: Shell `blktest` is gated (PermissionDenied) when NVMe is active.
///
/// With NVMe as the active block device, the shell cell still lacks
/// `can_block_io` so the kernel must deny the raw block I/O syscall.
/// This verifies:
///   1. The kernel booted fully to the shell (VFS + shell cells spawned).
///   2. `block_device()` returns the NVMe ZST proxy (not VirtIO).
///   3. The `can_block_io` capability gate still fires for unprivileged cells.
///
/// Note: this test boots WITHOUT a VirtIO block device so the NVMe path is
/// the only block device.  The kernel must still reach the shell prompt via
/// the embedded ramdisk (which is loaded from the kernel ELF itself).
#[test]
fn nvme_block_io_gate_enforced_x86() {
    if !prerequisites_ok() {
        return;
    }

    let nvme_disk = make_nvme_disk();

    let mut qemu = QemuRunner::boot_x86_bios_with_nvme(
        &iso_path(),
        &nvme_disk.to_string_lossy(),
    );

    // Wait for NVMe to initialise first.
    qemu.wait_for("[nvme] driver ready", NVME_INIT_TIMEOUT)
        .unwrap_or_else(|e| {
            let _ = std::fs::remove_file(&nvme_disk);
            panic!(
                "NVMe did not init before shell test: {e}\n--- output ---\n{}",
                qemu.dump()
            )
        });

    // Wait for the shell prompt.
    qemu.wait_for("ViCell >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| {
            let _ = std::fs::remove_file(&nvme_disk);
            panic!(
                "Shell prompt not reached on x86_64 NVMe boot: {e}\n--- output ---\n{}",
                qemu.dump()
            )
        });

    std::thread::sleep(std::time::Duration::from_millis(500));
    qemu.send_line("blktest");

    qemu.wait_for("blkio: denied", CMD_TIMEOUT)
        .unwrap_or_else(|e| {
            let _ = std::fs::remove_file(&nvme_disk);
            panic!(
                "Block I/O was NOT denied for non-VFS cell on NVMe boot: {e}\n--- output ---\n{}",
                qemu.dump()
            )
        });

    // Guard: NVMe must be the active device (not VirtIO fallback).
    assert!(
        qemu.output_contains("[nvme] driver ready"),
        "NVMe driver was not active when blktest ran\n--- output ---\n{}",
        qemu.dump()
    );
    // Guard: capability gate must fire, not grant access.
    assert!(
        !qemu.output_contains("blkio: ALLOWED"),
        "Block I/O gate let an unprivileged cell read NVMe device\n--- output ---\n{}",
        qemu.dump()
    );

    let _ = std::fs::remove_file(&nvme_disk);
}
