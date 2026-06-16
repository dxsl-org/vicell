//! Bootloader-handoff integration tests.
//!
//! Each test covers the kernel's early-init sequence — from the bootloader
//! handing off to `_start` through `kmain()` completing all subsystem init —
//! for each supported architecture.
//!
//! Tests only wait for boot-phase markers (≤ 15 s) rather than the full
//! service chain (shell prompt, 40 s). They are independent of disk images
//! and service cells: the minimal QEMU command lines attach no VirtIO devices,
//! so the tests run even before a `disk_v3.img` is generated.
//!
//! Prerequisites per arch:
//!   RV64 : qemu-system-riscv64 + `cargo build --release -p vicell-kernel`
//!   ARM64: qemu-system-aarch64 + `cargo build --target aarch64-unknown-none -p vicell-kernel --release`
//!   RV32 : qemu-system-riscv32 + `cargo build --target riscv32imc-unknown-none-elf -p vicell-kernel --release`
//!   x86  : `cargo build --target x86_64-unknown-none -p vicell-kernel --release` (no QEMU needed)

use std::path::PathBuf;
use vicell_integration_tests::{
    qemu_binary, qemu_binary_aarch64, qemu_binary_arm32, qemu_binary_i386,
    qemu_binary_rv32, qemu_binary_x86, QemuRunner,
};

/// Timeout for the handoff phase — must complete well before cells are spawned.
const HANDOFF_TIMEOUT: u64 = 15;

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root resolves")
}

fn rv64_kernel_path() -> String {
    repo_root()
        .join("target/riscv64gc-unknown-none-elf/release/vicell-kernel")
        .to_string_lossy()
        .into_owned()
}

fn aarch64_kernel_path() -> String {
    repo_root()
        .join("target/aarch64-unknown-none/release/vicell-kernel")
        .to_string_lossy()
        .into_owned()
}

fn rv32_kernel_path() -> String {
    repo_root()
        .join("target/riscv32imac-unknown-none-elf/release/vicell-kernel")
        .to_string_lossy()
        .into_owned()
}

fn aarch32_kernel_path() -> String {
    repo_root()
        .join("target/armv7a-none-eabi/release/vicell-kernel")
        .to_string_lossy()
        .into_owned()
}

fn x86_32_kernel_path() -> String {
    repo_root()
        .join("target/x86_32-unknown-none/release/vicell-kernel")
        .to_string_lossy()
        .into_owned()
}

fn x86_kernel_path() -> String {
    repo_root()
        .join("target/x86_64-unknown-none/release/vicell-kernel")
        .to_string_lossy()
        .into_owned()
}

fn x86_iso_path() -> String {
    repo_root()
        .join("build/vicell-x86.iso")
        .to_string_lossy()
        .into_owned()
}

// ---------------------------------------------------------------------------
// Prerequisite guards — skip gracefully instead of failing on missing tooling
// ---------------------------------------------------------------------------

fn rv64_prerequisites_ok() -> bool {
    let kernel_ok = PathBuf::from(rv64_kernel_path()).exists();
    let qemu_ok = std::process::Command::new(qemu_binary())
        .arg("--version")
        .output()
        .is_ok();
    if !kernel_ok {
        eprintln!("SKIP: RV64 kernel not built ({})", rv64_kernel_path());
    }
    if !qemu_ok {
        eprintln!("SKIP: qemu-system-riscv64 not on PATH");
    }
    kernel_ok && qemu_ok
}

fn aarch64_prerequisites_ok() -> bool {
    let kernel_ok = PathBuf::from(aarch64_kernel_path()).exists();
    let qemu_ok = std::process::Command::new(qemu_binary_aarch64())
        .arg("--version")
        .output()
        .is_ok();
    if !kernel_ok {
        eprintln!(
            "SKIP: AArch64 kernel not built ({})\n  build: cargo build --target aarch64-unknown-none -p vicell-kernel --release",
            aarch64_kernel_path()
        );
    }
    if !qemu_ok {
        eprintln!("SKIP: qemu-system-aarch64 not on PATH");
    }
    kernel_ok && qemu_ok
}

fn rv32_prerequisites_ok() -> bool {
    let kernel_ok = PathBuf::from(rv32_kernel_path()).exists();
    let qemu_ok = std::process::Command::new(qemu_binary_rv32())
        .arg("--version")
        .output()
        .is_ok();
    if !kernel_ok {
        eprintln!(
            "SKIP: RV32 kernel not built ({})\n  build: cargo build --target riscv32imac-unknown-none-elf -p vicell-kernel --release",
            rv32_kernel_path()
        );
    }
    if !qemu_ok {
        eprintln!("SKIP: qemu-system-riscv32 not on PATH");
    }
    kernel_ok && qemu_ok
}

fn aarch32_prerequisites_ok() -> bool {
    let kernel_ok = PathBuf::from(aarch32_kernel_path()).exists();
    let qemu_ok = std::process::Command::new(qemu_binary_arm32())
        .arg("--version")
        .output()
        .is_ok();
    if !kernel_ok {
        eprintln!(
            "SKIP: AArch32 kernel not built ({})\n  build: cargo build --target armv7a-none-eabi -p vicell-kernel --release",
            aarch32_kernel_path()
        );
    }
    if !qemu_ok {
        eprintln!("SKIP: qemu-system-arm not on PATH");
    }
    kernel_ok && qemu_ok
}

fn x86_32_prerequisites_ok() -> bool {
    let kernel_ok = PathBuf::from(x86_32_kernel_path()).exists();
    let qemu_ok = std::process::Command::new(qemu_binary_i386())
        .arg("--version")
        .output()
        .is_ok();
    if !kernel_ok {
        eprintln!(
            "SKIP: x86_32 kernel not built ({})\n  build: cargo build --target kernel/x86_32-unknown-none.json -p vicell-kernel --release -Z build-std=core,alloc",
            x86_32_kernel_path()
        );
    }
    if !qemu_ok {
        eprintln!("SKIP: qemu-system-i386 not on PATH");
    }
    kernel_ok && qemu_ok
}

// ---------------------------------------------------------------------------
// Phase 01 — RV64 handoff regression tests
//
// Verifies the canonical boot path: OpenSBI → PIE self-reloc → kmain →
// Limine-or-fallback BootInfo → frame alloc → paging → heap → HAL → scheduler.
// ---------------------------------------------------------------------------

/// Kernel prints its version banner immediately after _start completes.
#[test]
fn handoff_rv64_kernel_starts() {
    if !rv64_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_rv64(&rv64_kernel_path());
    qemu.wait_for("[ViCell] kernel boot v", 8)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

/// `kernel_phys_base=0x0000000080…` confirms parse_bootloader_info() ran and
/// returned a base within the RISC-V virt RAM window (0x8000_0000+).
/// The kernel prints a 16-digit zero-padded hex, so the prefix is `0x00000000`.
#[test]
fn handoff_rv64_phys_base() {
    if !rv64_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_rv64(&rv64_kernel_path());
    qemu.wait_for("kernel_phys_base=0x0000000080", 10)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

/// Frame allocator must initialise from the memory map within 12 s.
#[test]
fn handoff_rv64_frame_allocator() {
    if !rv64_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_rv64(&rv64_kernel_path());
    qemu.wait_for("Frame allocator initialized", 12)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

/// Page-table construction and activation must complete — confirms the SV39
/// root table was built and `satp` was switched.
#[test]
fn handoff_rv64_paging() {
    if !rv64_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_rv64(&rv64_kernel_path());
    qemu.wait_for("Paging activated", 13)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

/// Heap must be live (required before any `alloc` use) within the handoff window.
#[test]
fn handoff_rv64_heap() {
    if !rv64_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_rv64(&rv64_kernel_path());
    qemu.wait_for("Heap initialized", HANDOFF_TIMEOUT)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

// ---------------------------------------------------------------------------
// Phase 02 — AArch64 handoff tests
//
// Verifies EL2→EL1 transition + PL011 UART init + kmain subsystem init on
// the ARM virt machine. No disk; kernel falls back to embedded ramdisk.
// ---------------------------------------------------------------------------

/// AArch64 kernel must print its banner after EL1 entry and PL011 init.
#[test]
fn handoff_aarch64_kernel_starts() {
    if !aarch64_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_aarch64(&aarch64_kernel_path());
    qemu.wait_for("[ViCell] kernel boot v", 10)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

/// Physical base for AArch64 virt is at 0x4000_0000; kernel loads at 0x4008_0000.
/// The kernel prints a 16-digit zero-padded hex, so the prefix is `0x00000000`.
#[test]
fn handoff_aarch64_phys_base() {
    if !aarch64_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_aarch64(&aarch64_kernel_path());
    qemu.wait_for("kernel_phys_base=0x0000000040", 12)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

/// Frame allocator must initialise on AArch64 within the handoff window.
#[test]
fn handoff_aarch64_frame_allocator() {
    if !aarch64_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_aarch64(&aarch64_kernel_path());
    qemu.wait_for("Frame allocator initialized", 13)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

/// Heap must be live on AArch64 — confirms the paging + frame-alloc chain
/// works end-to-end before any cell is spawned.
#[test]
fn handoff_aarch64_heap() {
    if !aarch64_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_aarch64(&aarch64_kernel_path());
    qemu.wait_for("Heap initialized", HANDOFF_TIMEOUT)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

// ---------------------------------------------------------------------------
// Phase 03 — RV32 (Phase-31 Nano) handoff tests
//
// Verifies the OpenSBI → RV32 S-mode kernel path with SATP=0 (no page tables).
// No disk or peripheral devices are attached.
// ---------------------------------------------------------------------------

/// RV32 Nano must print its banner over the OpenSBI serial console.
#[test]
fn handoff_rv32_kernel_starts() {
    if !rv32_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_rv32(&rv32_kernel_path());
    qemu.wait_for("[ViCell] kernel boot v", 10)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

/// RV32 Nano uses bare physical addressing (SATP=0). Verify the correct
/// paging path was taken, distinct from the RV64 page-table build.
#[test]
fn handoff_rv32_bare_paging() {
    if !rv32_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_rv32(&rv32_kernel_path());
    qemu.wait_for("bare physical", 12)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

/// Heap must initialise on RV32 before the scheduler starts.
#[test]
fn handoff_rv32_heap() {
    if !rv32_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_rv32(&rv32_kernel_path());
    qemu.wait_for("Heap initialized", HANDOFF_TIMEOUT)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

// ---------------------------------------------------------------------------
// Phase 04 — x86_64 handoff tests
//
// Limine BIOS boot via ISO (build/vicell-x86.iso). Limine is configured with
// `timeout: 0` + `serial: yes` so COM1 output is immediate. The kernel runs
// on q35 SeaBIOS; paging is deferred (Limine PML4 used as-is for Phase 09).
//
// Rebuild ISO when the kernel changes:
//   wsl bash /mnt/d/ViCell/build/make-iso.sh
//
// AArch32 note: hal/arch/arm/src/aarch32/ exists but boot.rs is not yet
// implemented (context switch is unimplemented!). No boot tests until
// AArch32 boot.rs + kmain support lands.
// ---------------------------------------------------------------------------

fn x86_prerequisites_ok() -> bool {
    let iso_ok = PathBuf::from(x86_iso_path()).exists();
    let qemu_ok = std::process::Command::new(qemu_binary_x86())
        .arg("--version")
        .output()
        .is_ok();
    if !iso_ok {
        eprintln!(
            "SKIP: x86_64 ISO not built ({})\n  build: wsl bash /mnt/d/ViCell/build/make-iso.sh",
            x86_iso_path()
        );
    }
    if !qemu_ok {
        eprintln!("SKIP: qemu-system-x86_64 not on PATH");
    }
    iso_ok && qemu_ok
}

/// Verify the x86_64 kernel ELF was built and is a valid ELF binary.
/// Build-regression guard independent of ISO freshness.
#[test]
fn handoff_x86_kernel_builds() {
    let path = PathBuf::from(x86_kernel_path());
    if !path.exists() {
        eprintln!(
            "SKIP: x86_64 kernel not built ({})\n  build: cargo build --target x86_64-unknown-none -p vicell-kernel --release",
            x86_kernel_path()
        );
        return;
    }
    let meta = std::fs::metadata(&path).expect("stat x86_64 kernel");
    assert!(meta.len() > 0, "x86_64 kernel ELF is empty");
    let bytes = std::fs::read(&path).expect("read x86_64 kernel");
    assert_eq!(&bytes[..4], b"\x7fELF", "x86_64 kernel is not a valid ELF");
}

/// Limine must pass control to the kernel; the version banner appears on COM1.
/// Uses BIOS El Torito boot (SeaBIOS, no OVMF required).
#[test]
fn handoff_x86_kernel_starts() {
    if !x86_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_x86_bios(&x86_iso_path());
    qemu.wait_for("[ViCell] kernel boot v", 20)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

/// Limine provides a real memory map; `parse_bootloader_info()` must succeed
/// and log the physical base address via `[boot] kernel_phys_base=`.
#[test]
fn handoff_x86_phys_base() {
    if !x86_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_x86_bios(&x86_iso_path());
    qemu.wait_for("kernel_phys_base=0x", 22)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

/// Frame allocator must initialise from the Limine memory map on x86_64.
#[test]
fn handoff_x86_frame_allocator() {
    if !x86_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_x86_bios(&x86_iso_path());
    qemu.wait_for("Frame allocator initialized", 25)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

/// x86_64 bring-up uses Limine's PML4 (own tables deferred to Phase 09).
/// Verify the correct paging path was taken.
#[test]
fn handoff_x86_limine_paging() {
    if !x86_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_x86_bios(&x86_iso_path());
    qemu.wait_for("Paging: using Limine PML4", 27)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

/// Heap must be live on x86_64 — confirms phys_to_virt(HHDM) is correct.
#[test]
fn handoff_x86_heap() {
    if !x86_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_x86_bios(&x86_iso_path());
    qemu.wait_for("Heap initialized", 30)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

// ---------------------------------------------------------------------------
// Phase 05 — AArch32 (ARMv7-A) Nano handoff tests
//
// Direct `-kernel` boot into SVC mode at 0x40080000 (QEMU virt machine).
// MMU off, PL011 UART at 0x09000000. Bare physical addressing (no page tables).
// No VirtIO devices — kernel idles after "Scheduler initialized".
//
// Build: cargo build --target armv7a-none-eabi -p vicell-kernel --release
// QEMU:  qemu-system-arm (4.x+)
// ---------------------------------------------------------------------------

/// AArch32 Nano must print the version banner via PL011 UART.
#[test]
fn handoff_aarch32_kernel_starts() {
    if !aarch32_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_aarch32(&aarch32_kernel_path());
    qemu.wait_for("[ViCell] kernel boot v", 12)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

/// AArch32 uses bare physical addressing (MMU off). Verify the correct
/// code path was taken — distinct from the AArch64 page-table build.
#[test]
fn handoff_aarch32_bare_paging() {
    if !aarch32_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_aarch32(&aarch32_kernel_path());
    qemu.wait_for("bare physical", 14)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

/// Heap must initialise on AArch32 before the scheduler is ready.
#[test]
fn handoff_aarch32_heap() {
    if !aarch32_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_aarch32(&aarch32_kernel_path());
    qemu.wait_for("Heap initialized", HANDOFF_TIMEOUT)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

/// Scheduler must reach its idle loop on AArch32.
#[test]
fn handoff_aarch32_scheduler_initialized() {
    if !aarch32_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_aarch32(&aarch32_kernel_path());
    qemu.wait_for("Scheduler initialized", HANDOFF_TIMEOUT)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

// ---------------------------------------------------------------------------
// Phase 06 — x86_32 (IA-32) Nano handoff tests
//
// Multiboot1 boot via QEMU `-kernel` on `pc` machine. BIOS hands off to
// `_start` in protected mode; GDT/IDT initialised, COM1 UART at 0x3F8.
// Bare physical addressing (CR0.PG=0). No VirtIO — kernel idles after init.
//
// Build: cargo build --target kernel/x86_32-unknown-none.json -p vicell-kernel --release -Z build-std=core,alloc
// QEMU:  qemu-system-i386 (4.x+)
// ---------------------------------------------------------------------------

/// x86_32 Nano kernel must print its version banner over COM1.
#[test]
fn handoff_x86_32_kernel_starts() {
    if !x86_32_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_x86_32(&x86_32_kernel_path());
    qemu.wait_for("[ViCell] kernel boot v", 12)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

/// x86_32 uses bare physical addressing (CR0.PG=0). Verify the correct
/// code path was taken.
#[test]
fn handoff_x86_32_bare_paging() {
    if !x86_32_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_x86_32(&x86_32_kernel_path());
    qemu.wait_for("bare physical", 14)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

/// Heap must initialise on x86_32 before the scheduler starts.
#[test]
fn handoff_x86_32_heap() {
    if !x86_32_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_x86_32(&x86_32_kernel_path());
    qemu.wait_for("Heap initialized", HANDOFF_TIMEOUT)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}

/// Scheduler must reach its idle loop on x86_32.
#[test]
fn handoff_x86_32_scheduler_initialized() {
    if !x86_32_prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_x86_32(&x86_32_kernel_path());
    qemu.wait_for("Scheduler initialized", HANDOFF_TIMEOUT)
        .unwrap_or_else(|e| panic!("{e}\n--- output ---\n{}", qemu.dump()));
}
