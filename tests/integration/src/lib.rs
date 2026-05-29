//! ViOS integration-test harness.
//!
//! `QemuRunner` spawns `qemu-system-riscv64`, captures serial output on a
//! background thread (so `wait_for` can be called repeatedly), and can inject
//! input into the guest serial console via `send_line`.
//!
//! The default QEMU command line mirrors `run.ps1`: 128 MB RAM, the VirtIO
//! block device backed by `disk_v3.img`, a user-mode NIC and a VirtIO
//! keyboard. The GPU is intentionally omitted (its framebuffer setup currently
//! blocks the boot — see run.ps1).

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Resolve the qemu-system-riscv64 binary.
///
/// Order: `$VIOS_QEMU` env override → bare name on PATH → the default Windows
/// install location (`C:\Program Files\qemu\...`), mirroring run.ps1.
pub fn qemu_binary() -> String {
    if let Ok(p) = std::env::var("VIOS_QEMU") {
        if !p.is_empty() {
            return p;
        }
    }
    // Probe bare name on PATH.
    if Command::new("qemu-system-riscv64")
        .arg("--version")
        .output()
        .is_ok()
    {
        return "qemu-system-riscv64".to_string();
    }
    // Windows default install path fallback.
    let win = r"C:\Program Files\qemu\qemu-system-riscv64.exe";
    if Path::new(win).exists() {
        return win.to_string();
    }
    "qemu-system-riscv64".to_string()
}

/// QEMU-driven ViOS integration test runner.
pub struct QemuRunner {
    child: Child,
    stdin: Option<ChildStdin>,
    /// Lines captured from the guest serial output so far.
    output: Arc<Mutex<Vec<String>>>,
}

impl QemuRunner {
    /// Spawn QEMU booting `kernel` with `disk` attached as the VirtIO block
    /// device. Serial output is captured on a background reader thread.
    ///
    /// `kernel` and `disk` are paths relative to the current working directory
    /// (typically the repo root).
    pub fn boot(kernel: &str, disk: &str) -> Self {
        let mut child = Command::new(qemu_binary())
            .args([
                "-machine", "virt",
                "-m", "128M",
                "-nographic",
                "-bios", "default",
                "-kernel", kernel,
                "-drive", &format!("file={disk},format=raw,id=hd0,if=none"),
                "-device", "virtio-blk-device,drive=hd0",
                "-netdev", "user,id=net0",
                "-device", "virtio-net-device,netdev=net0",
                "-device", "virtio-keyboard-device",
                "-monitor", "none",
                "-serial", "stdio",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("qemu-system-riscv64 must be on PATH");

        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("stdout piped");
        let output = Arc::new(Mutex::new(Vec::<String>::new()));

        // Background reader: append every serial line to the shared buffer.
        let buf = Arc::clone(&output);
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(l) => buf.lock().unwrap().push(l),
                    Err(_) => break,
                }
            }
        });

        Self { child, stdin, output }
    }

    /// Block until any captured line contains `pattern`, or `timeout_secs`
    /// elapses. Returns the matching line on success.
    pub fn wait_for(&self, pattern: &str, timeout_secs: u64) -> Result<String, String> {
        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        loop {
            if let Some(hit) = self
                .output
                .lock()
                .unwrap()
                .iter()
                .find(|l| l.contains(pattern))
                .cloned()
            {
                return Ok(hit);
            }
            if Instant::now() > deadline {
                return Err(format!(
                    "timeout: pattern {:?} not seen in {}s",
                    pattern, timeout_secs
                ));
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Send `line` (a newline is appended) to the guest serial console.
    pub fn send_line(&mut self, line: &str) {
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = stdin.write_all(line.as_bytes());
            let _ = stdin.write_all(b"\n");
            let _ = stdin.flush();
        }
    }

    /// True if any captured line contains `needle`.
    pub fn output_contains(&self, needle: &str) -> bool {
        self.output.lock().unwrap().iter().any(|l| l.contains(needle))
    }

    /// Full captured output joined by newlines (for diagnostics on failure).
    pub fn dump(&self) -> String {
        self.output.lock().unwrap().join("\n")
    }
}

impl Drop for QemuRunner {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
