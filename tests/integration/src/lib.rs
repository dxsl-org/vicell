//! ViCell integration-test harness.
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
/// Order: `$ViCell_QEMU` env override → bare name on PATH → the default Windows
/// install location (`C:\Program Files\qemu\...`), mirroring run.ps1.
pub fn qemu_binary() -> String {
    if let Ok(p) = std::env::var("ViCell_QEMU") {
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

/// Resolve the qemu-system-aarch64 binary.
///
/// Order: `$ViCell_QEMU_AARCH64` env override → bare name on PATH → Windows default.
pub fn qemu_binary_aarch64() -> String {
    if let Ok(p) = std::env::var("ViCell_QEMU_AARCH64") {
        if !p.is_empty() {
            return p;
        }
    }
    if Command::new("qemu-system-aarch64")
        .arg("--version")
        .output()
        .is_ok()
    {
        return "qemu-system-aarch64".to_string();
    }
    let win = r"C:\Program Files\qemu\qemu-system-aarch64.exe";
    if Path::new(win).exists() {
        return win.to_string();
    }
    "qemu-system-aarch64".to_string()
}

/// Resolve the qemu-system-x86_64 binary.
///
/// Order: `$ViCell_QEMU_X86` env override → bare name on PATH → Windows default.
pub fn qemu_binary_x86() -> String {
    if let Ok(p) = std::env::var("ViCell_QEMU_X86") {
        if !p.is_empty() {
            return p;
        }
    }
    if Command::new("qemu-system-x86_64")
        .arg("--version")
        .output()
        .is_ok()
    {
        return "qemu-system-x86_64".to_string();
    }
    let win = r"C:\Program Files\qemu\qemu-system-x86_64.exe";
    if Path::new(win).exists() {
        return win.to_string();
    }
    "qemu-system-x86_64".to_string()
}

/// Resolve the qemu-system-riscv32 binary.
///
/// Order: `$ViCell_QEMU_RV32` env override → bare name on PATH → Windows default.
pub fn qemu_binary_rv32() -> String {
    if let Ok(p) = std::env::var("ViCell_QEMU_RV32") {
        if !p.is_empty() {
            return p;
        }
    }
    if Command::new("qemu-system-riscv32")
        .arg("--version")
        .output()
        .is_ok()
    {
        return "qemu-system-riscv32".to_string();
    }
    let win = r"C:\Program Files\qemu\qemu-system-riscv32.exe";
    if Path::new(win).exists() {
        return win.to_string();
    }
    "qemu-system-riscv32".to_string()
}

/// Resolve the qemu-system-arm binary (AArch32 / ARMv7-A).
///
/// Order: `$ViCell_QEMU_ARM32` env override → bare name on PATH → Windows default.
pub fn qemu_binary_arm32() -> String {
    if let Ok(p) = std::env::var("ViCell_QEMU_ARM32") {
        if !p.is_empty() {
            return p;
        }
    }
    if Command::new("qemu-system-arm")
        .arg("--version")
        .output()
        .is_ok()
    {
        return "qemu-system-arm".to_string();
    }
    let win = r"C:\Program Files\qemu\qemu-system-arm.exe";
    if Path::new(win).exists() {
        return win.to_string();
    }
    "qemu-system-arm".to_string()
}

/// Resolve the qemu-system-i386 binary (x86_32 / IA-32).
///
/// Order: `$ViCell_QEMU_I386` env override → bare name on PATH → Windows default.
pub fn qemu_binary_i386() -> String {
    if let Ok(p) = std::env::var("ViCell_QEMU_I386") {
        if !p.is_empty() {
            return p;
        }
    }
    if Command::new("qemu-system-i386")
        .arg("--version")
        .output()
        .is_ok()
    {
        return "qemu-system-i386".to_string();
    }
    let win = r"C:\Program Files\qemu\qemu-system-i386.exe";
    if Path::new(win).exists() {
        return win.to_string();
    }
    "qemu-system-i386".to_string()
}

/// QEMU-driven ViCell integration test runner.
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
    /// Temporary disk image path to delete on drop (None when using the shared disk).
    temp_disk: Option<std::path::PathBuf>,
    /// QEMU monitor TCP connection for sending monitor commands (e.g. `sendkey`).
    /// Only populated by `boot_with_netdev`; all other constructors set this to None.
    monitor: Option<TcpStream>,
}

impl QemuRunner {
    /// Spawn QEMU booting `kernel` with `disk` attached as the VirtIO block
    /// device, with the guest serial bridged to a localhost TCP socket.
    pub fn boot(kernel: &str, disk: &str) -> Self {
        Self::boot_with_netdev(kernel, disk, "user,id=net0")
    }

    /// Boot QEMU with a **private copy** of the disk image.
    ///
    /// Each call creates a unique temporary copy of `disk` so concurrent tests
    /// that write to the FAT16 partition cannot corrupt each other's data.  The
    /// copy is deleted when this `QemuRunner` is dropped.
    ///
    /// Use for any test that writes to `/data/` (FAT16).  Tests that only write
    /// to `/tmp/` (VFS RamFS, in-memory) can use the shared `boot` instead.
    pub fn boot_with_fresh_disk(kernel: &str, disk: &str) -> Self {
        let tmp = std::env::temp_dir().join(format!(
            "ViCell_disk_{}_{}.img",
            std::process::id(),
            // Use a combination of PID + a monotonic discriminator so that
            // multiple tests in the same process get distinct names.
            {
                use std::sync::atomic::{AtomicU64, Ordering};
                static CTR: AtomicU64 = AtomicU64::new(0);
                CTR.fetch_add(1, Ordering::Relaxed)
            }
        ));
        std::fs::copy(disk, &tmp)
            .unwrap_or_else(|e| panic!("failed to copy disk image for test isolation: {e}"));
        let mut runner = Self::boot_with_netdev(kernel, &tmp.to_string_lossy(), "user,id=net0");
        runner.temp_disk = Some(tmp);
        runner
    }

    /// Take the temp disk path out of this runner so Drop does NOT delete it.
    ///
    /// Used by persistence tests: the caller is responsible for cleaning up.
    pub fn take_disk_path(&mut self) -> Option<std::path::PathBuf> {
        self.temp_disk.take()
    }

    /// Boot QEMU with a SLIRP hostfwd: `127.0.0.1:<host_port>` → guest `guest_port`.
    ///
    /// Returns `(runner, host_port)`. Host port is discovered by binding `:0` then
    /// dropping the listener so QEMU/SLIRP can bind it — a benign TOCTOU race
    /// acceptable in test environments.
    pub fn boot_with_hostfwd(kernel: &str, disk: &str, guest_port: u16) -> (Self, u16) {
        let probe = TcpListener::bind("127.0.0.1:0").expect("probe bind");
        let host_port = probe.local_addr().unwrap().port();
        drop(probe); // release so QEMU/SLIRP can bind it momentarily

        let netdev = format!("user,id=net0,hostfwd=tcp:127.0.0.1:{host_port}-:{guest_port}");
        (Self::boot_with_netdev(kernel, disk, &netdev), host_port)
    }

    /// Boot QEMU with a minimal RV64 configuration (no disk, no VirtIO peripherals).
    ///
    /// Suitable for handoff smoke tests that only need to observe early boot markers
    /// (`Frame allocator initialized`, `Heap initialized`, etc.) without requiring a
    /// pre-built `disk_v3.img`. The guest serial is bridged to a TCP socket as usual.
    pub fn boot_rv64(kernel: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind serial socket");
        let port = listener.local_addr().unwrap().port();

        let child = Command::new(qemu_binary())
            .args([
                "-machine", "virt",
                "-m", "256M",
                "-nographic",
                "-bios", "default",
                "-kernel", kernel,
                "-monitor", "none",
                "-serial", &format!("tcp:127.0.0.1:{port}"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("qemu-system-riscv64 must be on PATH");

        listener.set_nonblocking(false).expect("blocking listener");
        let stream = listener.accept().expect("QEMU did not connect to the serial socket").0;
        let writer = stream.try_clone().expect("clone serial stream");

        let output = Arc::new(Mutex::new(String::new()));
        let buf = Arc::clone(&output);
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

        Self { child, writer: Some(writer), output, temp_disk: None, monitor: None }
    }

    /// Boot QEMU RISC-V with a single VirtIO-BLK disk attached.
    ///
    /// The disk is used **directly** (no temp copy) — callers that need test
    /// isolation must copy the image themselves before passing the path.  This
    /// intentional design allows the persistence test to share one `NamedTempFile`
    /// across two sequential `QemuRunner` instances without extra infrastructure.
    pub fn boot_rv64_with_disk(kernel: &str, disk: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind serial socket");
        let port = listener.local_addr().unwrap().port();

        let child = Command::new(qemu_binary())
            .args([
                "-machine", "virt",
                "-m", "256M",
                "-nographic",
                "-bios", "default",
                "-kernel", kernel,
                "-drive", &format!("file={disk},format=raw,if=none,id=hd0"),
                "-device", "virtio-blk-device,drive=hd0",
                "-monitor", "none",
                "-serial", &format!("tcp:127.0.0.1:{port}"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("qemu-system-riscv64 must be on PATH");

        listener.set_nonblocking(false).expect("blocking listener");
        let stream = listener.accept().expect("QEMU did not connect to the serial socket").0;
        let writer = stream.try_clone().expect("clone serial stream");

        let output = Arc::new(Mutex::new(String::new()));
        let buf = Arc::clone(&output);
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

        Self { child, writer: Some(writer), output, temp_disk: None, monitor: None }
    }

    /// Boot QEMU with an AArch64 kernel (no disk, no netdev — bring-up mode).
    ///
    /// Uses the `virt` machine with `cortex-a57`. The kernel is expected to
    /// fall back to its embedded ramdisk since no VirtIO block is attached.
    /// The PL011 UART on QEMU `virt` is mapped to serial 0 → the TCP socket.
    pub fn boot_aarch64(kernel: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind serial socket");
        let port = listener.local_addr().unwrap().port();

        let child = Command::new(qemu_binary_aarch64())
            .args([
                "-machine", "virt",
                "-cpu", "cortex-a57",
                "-m", "256M",
                "-nographic",
                "-kernel", kernel,
                "-monitor", "none",
                "-serial", &format!("tcp:127.0.0.1:{port}"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("qemu-system-aarch64 must be on PATH");

        listener.set_nonblocking(false).expect("blocking listener");
        let stream = listener.accept().expect("QEMU did not connect to the serial socket").0;
        let writer = stream.try_clone().expect("clone serial stream");

        let output = Arc::new(Mutex::new(String::new()));
        let buf = Arc::clone(&output);
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

        Self { child, writer: Some(writer), output, temp_disk: None, monitor: None }
    }

    /// Boot QEMU with an AArch64 kernel AND a VirtIO block disk (full boot mode).
    ///
    /// Adds the full peripheral set needed to spawn all services (VirtIO block,
    /// net SLIRP, RNG) so init can load /bin/vfs, /bin/shell, etc. from the disk.
    /// The guest serial is bridged to a TCP socket for bidirectional I/O as usual.
    pub fn boot_aarch64_with_disk(kernel: &str, disk: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind serial socket");
        let port = listener.local_addr().unwrap().port();

        let child = Command::new(qemu_binary_aarch64())
            .args([
                "-machine", "virt",
                "-cpu", "cortex-a57",
                "-m", "256M",
                "-nographic",
                "-kernel", kernel,
                "-drive", &format!("if=none,file={disk},format=raw,id=hd0"),
                "-device", "virtio-blk-device,drive=hd0",
                "-netdev", "user,id=net0",
                "-device", "virtio-net-device,netdev=net0",
                // rng-random is not available on Windows QEMU; omit the VirtIO
                // RNG device — it is not required for kernel boot or shell tests.
                "-no-reboot",
                "-monitor", "none",
                "-serial", &format!("tcp:127.0.0.1:{port}"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("qemu-system-aarch64 must be on PATH");

        listener.set_nonblocking(false).expect("blocking listener");
        let stream = listener.accept().expect("QEMU did not connect to the serial socket").0;
        let writer = stream.try_clone().expect("clone serial stream");

        let output = Arc::new(Mutex::new(String::new()));
        let buf = Arc::clone(&output);
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

        Self { child, writer: Some(writer), output, temp_disk: None, monitor: None }
    }

    /// Boot QEMU with an x86_64 Limine BIOS ISO.
    ///
    /// Uses SeaBIOS (no OVMF required) + Limine BIOS El Torito boot. The ISO
    /// must be built via `build/make-iso.sh` (WSL). Limine is configured with
    /// `timeout: 0` so it boots immediately; `serial: yes` routes Limine output
    /// to the COM1 UART, which is bridged to the TCP socket.
    pub fn boot_x86_bios(iso: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind serial socket");
        let port = listener.local_addr().unwrap().port();

        let child = Command::new(qemu_binary_x86())
            .args([
                "-machine", "q35",
                "-cpu", "qemu64",
                "-m", "256M",
                "-nographic",
                "-cdrom", iso,
                "-boot", "d",
                "-no-reboot",
                "-monitor", "none",
                "-serial", &format!("tcp:127.0.0.1:{port}"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("qemu-system-x86_64 must be on PATH");

        listener.set_nonblocking(false).expect("blocking listener");
        let stream = listener.accept().expect("QEMU did not connect to the serial socket").0;
        let writer = stream.try_clone().expect("clone serial stream");

        let output = Arc::new(Mutex::new(String::new()));
        let buf = Arc::clone(&output);
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

        Self { child, writer: Some(writer), output, temp_disk: None, monitor: None }
    }

    /// Boot QEMU with a RISC-V 32-bit kernel (Phase-31 Nano, no disk, no VirtIO).
    ///
    /// Uses OpenSBI (`-bios default`) + S-mode kernel. SATP=0 (bare physical).
    /// No disk or peripheral devices are attached — the kernel idles after init.
    pub fn boot_rv32(kernel: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind serial socket");
        let port = listener.local_addr().unwrap().port();

        let child = Command::new(qemu_binary_rv32())
            .args([
                "-machine", "virt",
                "-m", "128M",
                "-nographic",
                "-bios", "default",
                "-kernel", kernel,
                "-monitor", "none",
                "-serial", &format!("tcp:127.0.0.1:{port}"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("qemu-system-riscv32 must be on PATH");

        listener.set_nonblocking(false).expect("blocking listener");
        let stream = listener.accept().expect("QEMU did not connect to the serial socket").0;
        let writer = stream.try_clone().expect("clone serial stream");

        let output = Arc::new(Mutex::new(String::new()));
        let buf = Arc::clone(&output);
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

        Self { child, writer: Some(writer), output, temp_disk: None, monitor: None }
    }

    /// Boot QEMU with an AArch32 (ARMv7-A) bare-metal kernel (Nano profile).
    ///
    /// Machine: `virt`, CPU: `cortex-a15`, MMU off, PL011 UART at 0x09000000.
    /// Kernel is loaded directly with `-kernel`; no firmware (SVC mode entry).
    pub fn boot_aarch32(kernel: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind serial socket");
        let port = listener.local_addr().unwrap().port();

        let child = Command::new(qemu_binary_arm32())
            .args([
                "-machine", "virt",
                "-cpu", "cortex-a15",
                "-m", "128M",
                "-nographic",
                "-kernel", kernel,
                "-monitor", "none",
                "-serial", &format!("tcp:127.0.0.1:{port}"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("qemu-system-arm must be on PATH");

        listener.set_nonblocking(false).expect("blocking listener");
        let stream = listener.accept().expect("QEMU did not connect to the serial socket").0;
        let writer = stream.try_clone().expect("clone serial stream");

        let output = Arc::new(Mutex::new(String::new()));
        let buf = Arc::clone(&output);
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

        Self { child, writer: Some(writer), output, temp_disk: None, monitor: None }
    }

    /// Boot QEMU with an x86_32 (IA-32) bare-metal kernel via Multiboot1.
    ///
    /// Machine: `pc`, CPU: `base`, paging disabled (CR0.PG=0).
    /// QEMU `-kernel` speaks Multiboot1 — the multiboot header in `.text.boot`
    /// is detected and the kernel entry is called in protected mode.
    pub fn boot_x86_32(kernel: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind serial socket");
        let port = listener.local_addr().unwrap().port();

        let child = Command::new(qemu_binary_i386())
            .args([
                "-machine", "pc",
                "-cpu", "base",
                "-m", "128M",
                "-nographic",
                "-kernel", kernel,
                "-monitor", "none",
                "-serial", &format!("tcp:127.0.0.1:{port}"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("qemu-system-i386 must be on PATH");

        listener.set_nonblocking(false).expect("blocking listener");
        let stream = listener.accept().expect("QEMU did not connect to the serial socket").0;
        let writer = stream.try_clone().expect("clone serial stream");

        let output = Arc::new(Mutex::new(String::new()));
        let buf = Arc::clone(&output);
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

        Self { child, writer: Some(writer), output, temp_disk: None, monitor: None }
    }

    /// Internal: boot QEMU with a caller-specified `-netdev` value.
    ///
    /// All other QEMU args are fixed to match `run.ps1`; only the netdev string
    /// changes between `boot` (plain SLIRP) and `boot_with_hostfwd`.
    ///
    /// Binds both a serial socket and a QEMU monitor socket so that callers can
    /// inject keystrokes via `send_qemu_key` (e.g. for the keyboard E2E test).
    /// QEMU connects to both as a client on start (server=off / no `server` flag).
    fn boot_with_netdev(kernel: &str, disk: &str, netdev: &str) -> Self {
        // Bind an ephemeral port on the host; QEMU connects to it as a serial
        // backend (server=off,nowait → QEMU is the client and connects on start).
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind serial socket");
        let port = listener.local_addr().unwrap().port();

        // Bind a second ephemeral port for the QEMU monitor.
        // QEMU will connect to it as a client at startup (same pattern as serial).
        let monitor_listener = TcpListener::bind("127.0.0.1:0").expect("bind monitor socket");
        let monitor_port = monitor_listener.local_addr().unwrap().port();

        let child = Command::new(qemu_binary())
            .args([
                "-machine", "virt",
                "-m", "256M",
                // Use `-display none` instead of `-nographic` so that QEMU
                // creates a proper graphical console for virtio-gpu + virtio-keyboard,
                // enabling input-send-event to route keyboard events to the guest.
                // `-nographic` routes serial to stdio but prevents graphical console
                // creation, making input-send-event a no-op for VirtIO keyboard.
                // Serial output is still captured via the TCP socket below.
                "-display", "none",
                "-bios", "default",
                "-kernel", kernel,
                "-drive", &format!("file={disk},format=raw,id=hd0,if=none"),
                "-device", "virtio-blk-device,drive=hd0",
                "-netdev", netdev,
                "-device", "virtio-net-device,netdev=net0",
                "-device", "virtio-keyboard-device",
                "-device", "virtio-gpu-device",
                // QMP for keyboard injection via input-send-event.
                "-qmp", &format!("tcp:127.0.0.1:{monitor_port}"),
                // Serial 0 → TCP socket (bidirectional, replaces -nographic stdio mux).
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

        // Accept QEMU's connection to our monitor socket.
        // QEMU connects to both sockets (serial + monitor) asynchronously on start.
        // Both listeners already have their ports bound before QEMU spawns, so
        // QEMU's connection requests are queued in the OS backlog.  We accept
        // them sequentially (serial first).  Accept in a thread with a 10-s
        // deadline so callers that don't use the monitor are unaffected if QEMU
        // for some reason never connects (graceful degradation).
        let monitor_stream = {
            let (tx, rx) = std::sync::mpsc::channel::<TcpStream>();
            thread::spawn(move || {
                monitor_listener.set_nonblocking(false).ok();
                if let Ok((s, _)) = monitor_listener.accept() {
                    let _ = tx.send(s);
                }
            });
            rx.recv_timeout(std::time::Duration::from_secs(10)).ok()
        };

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

        Self { child, writer: Some(writer), output, temp_disk: None, monitor: monitor_stream }
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

    /// Inject a keypress into the guest VirtIO keyboard via the QEMU monitor.
    ///
    /// `key` uses QEMU's `sendkey` syntax, e.g. `"tab"`, `"ret"`, `"a"`.
    /// The keypress is a single press+release event pair as generated by QEMU.
    ///
    /// Only functional when called on a runner created by `boot` / `boot_with_netdev`
    /// (the only constructors that wire up the monitor TCP socket). All other
    /// constructors have `monitor: None` and this method is a no-op.
    pub fn send_qemu_key(&mut self, key: &str) {
        let Some(m) = self.monitor.as_mut() else {
            eprintln!("[test] WARNING: QMP socket is None — sendkey {:?} dropped", key);
            return;
        };

        // QMP handshake: read the server greeting, then negotiate capabilities.
        // The greeting is sent immediately on connect.  Drain it now if we
        // haven't already (the timeout makes this idempotent on repeat calls).
        m.set_read_timeout(Some(std::time::Duration::from_millis(200))).ok();
        let mut buf = vec![0u8; 4096];
        loop {
            match m.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }

        // Capabilities negotiation — required before any QMP command.
        // Idempotent: QEMU silently ignores duplicate negotiation.
        m.set_read_timeout(None).ok();
        let _ = m.write_all(b"{\"execute\":\"qmp_capabilities\"}\n");
        let _ = m.flush();
        // Drain the {"return": {}} ack.
        m.set_read_timeout(Some(std::time::Duration::from_millis(300))).ok();
        loop {
            match m.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }

        let cmd = format!(
            concat!(
                r#"{{"execute":"input-send-event","arguments":{{"events":["#,
                r#"{{"type":"key","data":{{"down":true,"key":{{"type":"qcode","data":"{key}"}}}}}}"#,
                r#",{{"type":"key","data":{{"down":false,"key":{{"type":"qcode","data":"{key}"}}}}}}"#,
                r#"]}}}}"#,
            ),
            key = key,
        );
        eprintln!("[test] QMP input-send-event key={key:?}");
        let _ = m.write_all(cmd.as_bytes());
        let _ = m.write_all(b"\n");
        let _ = m.flush();
        // Read response for diagnostics.
        m.set_read_timeout(Some(std::time::Duration::from_millis(500))).ok();
        let mut resp = vec![0u8; 1024];
        match m.read(&mut resp) {
            Ok(n) if n > 0 => eprintln!("[test] QMP response: {:?}", String::from_utf8_lossy(&resp[..n])),
            Ok(_) => eprintln!("[test] QMP response: empty"),
            Err(e) => eprintln!("[test] QMP read error: {e}"),
        }
        m.set_read_timeout(None).ok();
    }

    /// Wait for QEMU to exit on its own (e.g. after a guest `shutdown` command).
    ///
    /// Returns `true` if the process exited within `timeout_secs`. On timeout the
    /// process is left running and `Drop` will SIGKILL it. Used by reboot-persistence
    /// tests so the VirtIO block backend can flush `disk_v3.img` before re-booting.
    ///
    /// Closes our serial writer first so QEMU's exit is not held open by a live
    /// TCP client; the background reader thread will then see EOF and stop.
    pub fn wait_for_natural_exit(&mut self, timeout_secs: u64) -> bool {
        // Release our half of the serial socket so QEMU can fully tear down.
        self.writer.take();

        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return true, // exited naturally — disk flushed
                Ok(None) => {}              // still running
                Err(_) => return false,     // wait failed — let Drop handle it
            }
            if Instant::now() > deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(100));
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
        // Remove the temporary disk copy created by `boot_with_fresh_disk`, if any.
        if let Some(ref p) = self.temp_disk {
            let _ = std::fs::remove_file(p);
        }
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

/// Resolve the qemu-system-x86_64 binary.
///
/// Order: `$VIOS_QEMU_X86` env override → bare name on PATH → Windows default.
pub fn qemu_x86_binary() -> String {
    if let Ok(p) = std::env::var("VIOS_QEMU_X86") {
        if !p.is_empty() {
            return p;
        }
    }
    if Command::new("qemu-system-x86_64")
        .arg("--version")
        .output()
        .is_ok()
    {
        return "qemu-system-x86_64".to_string();
    }
    let win = r"C:\Program Files\qemu\qemu-system-x86_64.exe";
    if Path::new(win).exists() {
        return win.to_string();
    }
    "qemu-system-x86_64".to_string()
}

impl QemuRunner {
    /// Boot an x86_64 q35 guest with an NVMe disk attached.
    ///
    /// Uses `qemu-system-x86_64 -machine q35` with a PCIe NVMe controller
    /// (`-device nvme`) backed by `nvme_disk`. The guest serial (COM1) is
    /// bridged to a localhost TCP socket the same way as `boot_with_netdev`.
    ///
    /// Prerequisites: `qemu-system-x86_64` on PATH (or `$VIOS_QEMU_X86`),
    /// a built x86_64 kernel, and a raw disk image for the NVMe drive.
    pub fn boot_x86_nvme(kernel: &str, nvme_disk: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind serial socket");
        let port = listener.local_addr().unwrap().port();

        let child = Command::new(qemu_x86_binary())
            .args([
                "-machine", "q35",
                "-cpu",     "qemu64",
                "-m",       "256M",
                "-nographic",
                // NVMe drive: PCIe NVMe controller backed by nvme_disk.
                "-drive",   &format!("file={nvme_disk},format=raw,if=none,id=nvme0"),
                "-device",  "nvme,drive=nvme0,serial=deadbeef01",
                // Serial → TCP socket (same pattern as boot_with_netdev).
                "-serial",  &format!("tcp:127.0.0.1:{port}"),
                "-kernel",  kernel,
                "-monitor", "none",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("qemu-system-x86_64 must be on PATH or set $VIOS_QEMU_X86");

        listener.set_nonblocking(false).expect("blocking listener");
        let stream = listener
            .accept()
            .expect("QEMU did not connect to the serial socket")
            .0;
        let writer = stream.try_clone().expect("clone serial stream");

        let output = Arc::new(Mutex::new(String::new()));
        let buf = Arc::clone(&output);
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

        Self { child, writer: Some(writer), output, temp_disk: None }
    }
}

/// Spawn a minimal MQTT 3.1.1 mock broker on an ephemeral port.
///
/// Protocol:
/// - Waits for a CONNECT packet (first byte 0x10), replies CONNACK `[20 02 00 00]`.
/// - If next packet is SUBSCRIBE (0x82): replies SUBACK then injects one PUBLISH
///   carrying `inject_payload` on a single-byte topic `"t"`.
/// - If next packet is PUBLISH (0x30): captures the payload and sends it on the
///   returned `Receiver`; useful for asserting what the client published.
///
/// Returns `(port, Receiver<Vec<u8>>)`. The receiver yields at most one item
/// (the PUBLISH payload) — or times out if no PUBLISH was sent.
/// The caller must keep the returned `JoinHandle` (inside `Receiver`) alive.
pub fn spawn_mqtt_broker(
    inject_payload: &'static [u8],
) -> (u16, std::sync::mpsc::Receiver<Vec<u8>>) {
    use std::sync::mpsc;
    let listener = TcpListener::bind("127.0.0.1:0").expect("mqtt broker bind");
    let port = listener.local_addr().expect("mqtt local addr").port();
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else { return };
        // Use a single buffer for all reads so CONNECT + SUBSCRIBE bytes
        // arriving in the same TCP segment are handled correctly.
        // `pos` tracks where the next packet starts after CONNECT is consumed.
        let mut buf = [0u8; 512];
        let mut filled = 0usize; // bytes in buf

        // Phase 1: accumulate until we have the full CONNECT packet.
        loop {
            match stream.read(&mut buf[filled..]) {
                Ok(0) | Err(_) => return,
                Ok(k) => {
                    filled += k;
                    if filled >= 2 && buf[0] == 0x10 {
                        // CONNECT remaining_len is always < 128 for our client.
                        if filled >= 2 + buf[1] as usize { break; }
                    }
                    if filled >= 512 { return; }
                }
            }
        }
        if buf[0] != 0x10 { return; }
        let _ = stream.write_all(&[0x20, 0x02, 0x00, 0x00]); // CONNACK

        // For subscribe tests: inject_payload is non-empty.  Send SUBACK + PUBLISH
        // proactively — we don't need to parse the client's SUBSCRIBE in a mock.
        // Small delay gives the client time to finish processing CONNACK and call
        // its recv loop before we send SUBACK, so both packets arrive in distinct
        // TCP segments (avoiding mqtt_recv truncation at the SUBACK boundary).
        if !inject_payload.is_empty() {
            // 50 ms lets the client finish processing CONNACK and enter its
            // SUBACK poll loop before SUBACK arrives — avoids a race where
            // mqtt_recv drains all 500 polls in ~50 ms before we send SUBACK.
            thread::sleep(std::time::Duration::from_millis(50));
            let _ = stream.write_all(&[0x90, 0x03, 0x00, 0x01, 0x00]); // SUBACK
            // 500 ms gives the client time to consume the SUBACK via RECV_OP and
            // start its PUBLISH poll loop before PUBLISH arrives.  Without this
            // gap, the net service may deliver SUBACK + PUBLISH in one RECV
            // response; mqtt_recv extracts only the first packet and discards the
            // trailing PUBLISH bytes, which are then lost from smoltcp's buffer.
            thread::sleep(std::time::Duration::from_millis(500));
            // PUBLISH: topic "t" (1 byte), payload = inject_payload.
            let topic = b"t";
            let remaining = 2 + topic.len() + inject_payload.len();
            let mut pkt = Vec::with_capacity(4 + remaining);
            pkt.push(0x30u8);
            pkt.push(remaining as u8);
            pkt.push(0x00);
            pkt.push(topic.len() as u8);
            pkt.extend_from_slice(topic);
            pkt.extend_from_slice(inject_payload);
            let _ = stream.write_all(&pkt);
            // Keep the connection alive so PUBLISH is fully delivered before
            // the socket closes.  Closing immediately (TcpStream drop) sends FIN
            // in the same TCP segment as PUBLISH on some OSes, which can cause
            // smoltcp to process FIN before the PUBLISH payload.
            thread::sleep(std::time::Duration::from_millis(1000));
            return; // subscribe-mode broker done
        }

        // For publish tests: read the client's PUBLISH packet and capture its payload.
        // Drain the rest of the CONNECT body first (it may or may not already be in buf).
        let connect_end = 2 + buf[1] as usize;
        // Keep reading until we have a full packet after connect_end.
        while filled < connect_end {
            match stream.read(&mut buf[filled..]) {
                Ok(0) | Err(_) => return,
                Ok(k) => { filled += k; }
            }
        }
        let pos = connect_end;
        loop {
            let have = filled.saturating_sub(pos);
            if have >= 2 && have >= 2 + buf[pos + 1] as usize { break; }
            match stream.read(&mut buf[filled..]) {
                Ok(0) | Err(_) => break,
                Ok(k) => { filled += k; }
            }
        }
        let next = &buf[pos..filled];
        let n    = next.len();
        if n > 0 && next[0] == 0x30 {
            // PUBLISH: extract payload after fixed-header(2) + topic_len(2) + topic.
            let remaining     = next[1] as usize;
            let topic_len     = (next[2] as usize) << 8 | next[3] as usize;
            let payload_start = 4 + topic_len;
            let payload_end   = (2 + remaining).min(n);
            if payload_end > payload_start {
                let _ = tx.send(next[payload_start..payload_end].to_vec());
            }
        }
    });
    (port, rx)
}
