//! Phase B-03/B-04: x86_64 PCIe NIC (e1000) + Intel VT-d integration tests.
//!
//! `nic_x86_e1000_init` — boots QEMU q35 with `-device e1000` and asserts the
//! NIC driver probe log. Verifies the ECAM scan, BAR mapping, EEPROM read, and
//! ring initialisation all succeed.
//!
//! `nic_x86_vtd_enabled` — same boot plus `-device intel-iommu`; asserts both
//! the VT-d passthrough log and the NIC init log.
//!
//! Both tests skip gracefully when the x86_64 ISO is not built or
//! `qemu-system-x86_64` is not on PATH.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use vicell_integration_tests::{qemu_x86_binary, QemuRunner};

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
    let iso_ok = PathBuf::from(iso_path()).exists();
    let qemu_ok = std::process::Command::new(qemu_x86_binary())
        .arg("--version")
        .output()
        .is_ok();

    if !iso_ok {
        eprintln!(
            "SKIP nic-x86: x86_64 ISO not built ({})\n\
             Build with: scripts/build-x86_64-cells.ps1 then build/make-iso.sh",
            iso_path()
        );
    }
    if !qemu_ok {
        eprintln!("SKIP nic-x86: qemu-system-x86_64 not on PATH");
    }
    iso_ok && qemu_ok
}

fn make_nvme_disk() -> PathBuf {
    static CTR: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "vicell_nic_x86_{}_{}.img",
        std::process::id(),
        CTR.fetch_add(1, Ordering::Relaxed)
    ));
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(&path)
        .expect("create NVMe disk image");
    f.set_len(64 * 1024 * 1024).expect("set NVMe disk size");
    let _ = f.write_all(b"");
    path
}

/// Phase B-03: e1000 NIC initialises on x86_64 q35.
///
/// Verifies ECAM finds the e1000 endpoint, BAR0 is mapped, EEPROM
/// gives a valid MAC, and the TX/RX rings are programmed.
#[test]
fn nic_x86_e1000_init() {
    if !prerequisites_ok() { return; }

    let disk = make_nvme_disk();
    let qemu = QemuRunner::boot_x86_bios_with_nic(&iso_path(), &disk.to_string_lossy());

    qemu.wait_for("[e1000] NIC initialized", BOOT_TIMEOUT)
        .unwrap_or_else(|e| {
            let _ = std::fs::remove_file(&disk);
            panic!(
                "e1000 init not seen within {BOOT_TIMEOUT}s: {e}\n\
                 Hint: check ECAM class filter (0x02/0x00/0x00) and BAR mapping.\n\
                 --- serial output ---\n{}",
                qemu.dump()
            )
        });

    let _ = std::fs::remove_file(&disk);
}

/// Phase B-04: Intel VT-d passthrough + e1000 on x86_64 q35.
///
/// Verifies the VT-d MMIO at 0xFED90000 is identity-mapped, GCAP is valid,
/// root/context tables are programmed, GCMD.TE succeeds, and the e1000
/// initialises after VT-d is active.
#[test]
fn nic_x86_vtd_enabled() {
    if !prerequisites_ok() { return; }

    let disk = make_nvme_disk();
    let qemu = QemuRunner::boot_x86_bios_with_vtd(&iso_path(), &disk.to_string_lossy());

    qemu.wait_for("[vtd] Intel VT-d passthrough enabled", BOOT_TIMEOUT)
        .unwrap_or_else(|e| {
            let _ = std::fs::remove_file(&disk);
            panic!(
                "VT-d not enabled within {BOOT_TIMEOUT}s: {e}\n\
                 Hint: -device intel-iommu must be in QEMU args before -device e1000.\n\
                 --- serial output ---\n{}",
                qemu.dump()
            )
        });

    // e1000 must also initialise after VT-d is active.
    qemu.wait_for("[e1000] NIC initialized", 10)
        .unwrap_or_else(|e| {
            let _ = std::fs::remove_file(&disk);
            panic!(
                "e1000 not init after VT-d: {e}\n--- serial output ---\n{}",
                qemu.dump()
            )
        });

    let _ = std::fs::remove_file(&disk);
}
