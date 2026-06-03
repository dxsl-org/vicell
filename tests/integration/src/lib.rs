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

use std::io::{BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
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
///
/// The guest serial port is exposed over a TCP socket (QEMU
/// `-serial tcp:...,server`) rather than stdio. A TCP byte stream is the
/// reliable channel for *bidirectional* automated serial I/O: piped stdio is
/// subject to host/QEMU buffering that can swallow injected keystrokes.
pub struct QemuRunner {
    child: Child,
    writer: Option<TcpStream>,
    /// Raw bytes captured from the guest serial output so far.
    output: Arc<Mutex<String>>,
}

impl QemuRunner {
    /// Spawn QEMU booting `kernel` with `disk` attached as the VirtIO block
    /// device, with the guest serial bridged to a localhost TCP socket.
    ///
    /// `kernel` and `disk` are paths relative to the current working directory
    /// (typically the repo root).
    pub fn boot(kernel: &str, disk: &str) -> Self {
        // Bind an ephemeral port on the host; QEMU connects to it as a serial
        // backend (server=off,nowait → QEMU is the client and connects on start).
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind serial socket");
        let port = listener.local_addr().unwrap().port();

        let child = Command::new(qemu_binary())
            .args([
                "-machine", "virt",
                "-m", "256M",
                "-nographic",
                "-bios", "default",
                "-kernel", kernel,
                "-drive", &format!("file={disk},format=raw,id=hd0,if=none"),
                "-device", "virtio-blk-device,drive=hd0",
                "-netdev", "user,id=net0",
                "-device", "virtio-net-device,netdev=net0",
                "-device", "virtio-keyboard-device",
                "-device", "virtio-gpu-device",
                "-monitor", "none",
                "-serial", &format!("tcp:127.0.0.1:{port}"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("qemu-system-riscv64 must be on PATH");

        // Accept QEMU's connection to our serial socket.
        listener
            .set_nonblocking(false)
            .expect("blocking listener");
        let stream = listener
            .accept()
            .expect("QEMU did not connect to the serial socket")
            .0;
        let writer = stream.try_clone().expect("clone serial stream");

        let output = Arc::new(Mutex::new(String::new()));
        let buf = Arc::clone(&output);
        // Background reader: append all serial bytes to the shared buffer.
        thread::spawn(move || {
            let mut reader = BufReader::new(stream);
            let mut byte = [0u8; 1];
            loop {
                match reader.read(&mut byte) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => buf.lock().unwrap().push(byte[0] as char),
                }
            }
        });

        Self { child, writer: Some(writer), output }
    }

    /// Block until any captured line contains `pattern`, or `timeout_secs`
    /// elapses. Returns the matching line on success.
    pub fn wait_for(&self, pattern: &str, timeout_secs: u64) -> Result<String, String> {
        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        loop {
            if self.output.lock().unwrap().contains(pattern) {
                return Ok(pattern.to_string());
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
        if let Some(w) = self.writer.as_mut() {
            let _ = w.write_all(line.as_bytes());
            let _ = w.write_all(b"\n");
            let _ = w.flush();
        }
    }

    /// True if the captured serial output contains `needle`.
    pub fn output_contains(&self, needle: &str) -> bool {
        self.output.lock().unwrap().contains(needle)
    }

    /// Full captured serial output (for diagnostics on failure).
    pub fn dump(&self) -> String {
        self.output.lock().unwrap().clone()
    }
}

impl Drop for QemuRunner {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Spawn a single-connection HTTP/1.0 server on an ephemeral loopback port.
///
/// Reads request headers (until `\r\n\r\n`), replies a fixed
/// `HTTP/1.0 200 OK\r\nContent-Length: 5\r\n\r\nHELLO`, then drops the stream
/// to send FIN. QEMU SLIRP routes guest→`10.0.2.2:port` to host→`127.0.0.1:port`.
///
/// Returns `(port, handle)`. The caller **must** keep `handle` alive for the
/// test duration so the server thread outlives the QEMU session.
pub fn spawn_http_server() -> (u16, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("http server bind");
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            let mut total = 0usize;
            loop {
                match stream.read(&mut buf[total..]) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        total += n;
                        if buf[..total].windows(4).any(|w| w == b"\r\n\r\n") { break; }
                        if total == buf.len() { break; }
                    }
                }
            }
            let _ = stream.write_all(
                b"HTTP/1.0 200 OK\r\nContent-Length: 5\r\n\r\nHELLO",
            );
            drop(stream); // sends FIN — curl's SOCKET_STATE will see CloseWait
        }
    });
    (port, handle)
}

/// Spawn a single-connection TCP echo server on an ephemeral loopback port.
///
/// Returns the bound port. QEMU SLIRP routes guest→`10.0.2.2:port` to
/// host→`127.0.0.1:port`, so the guest's nc can reach this server.
/// The server exits after handling one connection (sufficient for tests).
pub fn spawn_echo_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("echo server bind");
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 256];
            loop {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => { let _ = stream.write_all(&buf[..n]); }
                }
            }
        }
    });
    port
}
