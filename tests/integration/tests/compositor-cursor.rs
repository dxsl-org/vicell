//! Integration test: compositor software cursor end-to-end.
//!
//! Boots QEMU with a VirtIO GPU + VirtIO tablet device, injects an absolute
//! mouse-pointer event via QMP `input-send-event`, and asserts that the
//! compositor emits `[compositor] cursor at X,Y` on serial.
//!
//! Prerequisites: qemu-system-riscv64 on PATH, kernel + disk built.
//! Gracefully skips when any prerequisite is missing.

use std::path::PathBuf;
use std::time::Duration;

use vicell_integration_tests::{qemu_binary, QemuRunner};

const BOOT_TIMEOUT: u64 = 60;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root resolves")
}

fn kernel_path() -> String {
    repo_root()
        .join("target/riscv64gc-unknown-none-elf/release/vicell-kernel")
        .to_string_lossy()
        .into_owned()
}

fn disk_path() -> String {
    repo_root().join("disk_v3.img").to_string_lossy().into_owned()
}

fn prerequisites_ok() -> bool {
    let kernel_exists = PathBuf::from(kernel_path()).exists();
    let disk_exists   = PathBuf::from(disk_path()).exists();
    let qemu_ok = std::process::Command::new(qemu_binary())
        .arg("--version")
        .output()
        .is_ok();
    if !kernel_exists {
        eprintln!("SKIP compositor-cursor: kernel not built ({})", kernel_path());
    }
    if !disk_exists {
        eprintln!("SKIP compositor-cursor: disk_v3.img missing — run ./gen_disk.ps1");
    }
    if !qemu_ok {
        eprintln!("SKIP compositor-cursor: qemu-system-riscv64 not on PATH");
    }
    kernel_exists && disk_exists && qemu_ok
}

/// End-to-end cursor move test.
///
/// Data flow:
///   QMP abs event → QEMU virtio-tablet → kernel virtio_input (EV_ABS, opcode 2)
///   → input service apply_abs → MouseMove{x,y}
///   → compositor update_cursor → "[compositor] cursor at X,Y"
///
/// The QEMU abs coordinate 16383 maps to roughly the centre of the 32767-range
/// (display-independent). Any non-zero position is sufficient to assert the
/// cursor moved from its initial (0,0) position.
#[test]
fn compositor_cursor_moves_on_mouse_event() {
    if !prerequisites_ok() {
        return;
    }

    let mut qemu = QemuRunner::boot_with_pointer(&kernel_path(), &disk_path());

    // Wait for the shell prompt — full userspace stack is up by this point.
    qemu.wait_for("ViCell >", BOOT_TIMEOUT).unwrap_or_else(|e| {
        panic!(
            "shell not reached: {e}\n--- serial output ---\n{}",
            qemu.dump()
        )
    });

    // Wait for the compositor to print its startup banner.
    qemu.wait_for("[compositor] Compositor v0.2", 15).unwrap_or_else(|e| {
        panic!(
            "compositor did not start: {e}\n--- serial output ---\n{}",
            qemu.dump()
        )
    });

    // Give the compositor time to settle its input-focus registration loop.
    std::thread::sleep(Duration::from_millis(400));

    // Inject an absolute pointer move to near-centre of the QEMU logical range
    // (0..32767). Sending both axes in one call avoids split-event coalescing.
    // The VirtIO input ring is polled on the 10 ms timer tick; allow 15 s.
    qemu.send_qemu_mouse_abs(16383, 16383);

    // Leg 1: input service received EV_ABS (opcode 2 from kernel).
    qemu.wait_for("[input-svc] key event 2", 15).unwrap_or_else(|e| {
        panic!(
            "EV_ABS not received by input service: {e}\n\
             Hint: verify virtio-tablet-device is attached and input service handles opcode 2.\n\
             --- serial output ---\n{}",
            qemu.dump()
        )
    });

    // Leg 2: compositor received the MouseMove and drew the cursor.
    // The probe "[compositor] cursor at X,Y" is emitted by update_cursor in
    // input_handler.rs on every MouseMove.
    qemu.wait_for("[compositor] cursor at ", 10).unwrap_or_else(|e| {
        panic!(
            "compositor cursor probe not seen: {e}\n\
             Hint: verify update_cursor emits the probe and pending_dirty is set.\n\
             --- serial output ---\n{}",
            qemu.dump()
        )
    });

    // Verify the cursor moved from the initial (0,0) position — the reported
    // coords must contain at least one non-zero value.
    let output = qemu.dump();
    let probe_line = output
        .lines()
        .find(|l| l.contains("[compositor] cursor at "))
        .expect("cursor probe line must be present after wait_for succeeded");

    // Coords follow "cursor at " — format is "X,Y".
    let coords_str = probe_line
        .split("[compositor] cursor at ")
        .nth(1)
        .unwrap_or("")
        .trim();
    let not_origin = coords_str != "0,0";
    assert!(
        not_origin,
        "cursor stayed at origin — EV_ABS event may not have reached compositor (coords={coords_str:?})"
    );

    eprintln!("[test] cursor probe: {:?}", probe_line);
}
