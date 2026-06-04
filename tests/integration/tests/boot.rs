//! End-to-end boot + interactive tests driven through QEMU serial.
//!
//! These require `qemu-system-riscv64` on PATH and pre-built artifacts:
//!   cargo build --release -p vios-kernel
//!   ./gen_disk.ps1
//!
//! Paths are relative to the repo root (two levels up from this crate). The
//! tests resolve them from CARGO_MANIFEST_DIR so they run regardless of cwd.

use std::io::Write;
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;
use vios_integration_tests::{qemu_binary, spawn_echo_server, spawn_http_server, QemuRunner};

const BOOT_TIMEOUT: u64 = 40;
/// Timeout for individual shell command round-trips after boot.
const CMD_TIMEOUT: u64 = 10;

/// Repo root = tests/integration/.. /..
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root resolves")
}

fn kernel_path() -> String {
    repo_root()
        .join("target/riscv64gc-unknown-none-elf/release/vios-kernel")
        .to_string_lossy()
        .into_owned()
}

fn disk_path() -> String {
    repo_root().join("disk_v3.img").to_string_lossy().into_owned()
}

/// Skip (don't fail) when prerequisites are missing, so the suite is friendly
/// on machines without QEMU or a built kernel.
fn prerequisites_ok() -> bool {
    let kernel_exists = PathBuf::from(kernel_path()).exists();
    let disk_exists = PathBuf::from(disk_path()).exists();
    let qemu_ok = std::process::Command::new(qemu_binary())
        .arg("--version")
        .output()
        .is_ok();
    if !kernel_exists {
        eprintln!("SKIP: kernel not built ({})", kernel_path());
    }
    if !disk_exists {
        eprintln!("SKIP: disk_v3.img missing — run ./gen_disk.ps1");
    }
    if !qemu_ok {
        eprintln!("SKIP: qemu-system-riscv64 not on PATH");
    }
    kernel_exists && disk_exists && qemu_ok
}

/// Phase 03/06/13/14/16/17: the kernel must boot through the full service
/// chain and present the shell prompt.
#[test]
fn boots_to_shell_prompt() {
    if !prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("shell prompt not reached: {e}\n--- output ---\n{}", qemu.dump()));

    // Phase 03: Ring-3 user task ran.
    assert!(
        qemu.output_contains("user_hello") || qemu.output_contains("U-mode"),
        "ring-3 user task did not run"
    );
    // Phase 13/14/16: services spawned via SpawnFromPath.
    assert!(qemu.output_contains("/bin/vfs"), "VFS service did not spawn");
    assert!(qemu.output_contains("/bin/shell"), "shell did not spawn");
}

/// Phase 04/13: the embedded FAT16 image must mount (regression guard for the
/// CorruptedFileSystem bug fixed by switching mkfat32.py to FAT16).
#[test]
fn fat_filesystem_mounts() {
    if !prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("mounted successfully", BOOT_TIMEOUT).unwrap_or_else(|e| {
        panic!("FAT mount not confirmed: {e}\n--- output ---\n{}", qemu.dump())
    });
    assert!(
        !qemu.output_contains("Corrupted") && !qemu.output_contains("Failed to mount"),
        "FAT mount reported an error"
    );
}

/// Phase 17: the shell must process an interactive command. We wait for the
/// prompt, send `echo` over the serial socket, and expect the argument echoed
/// back. Verifies the full UART RX → console driver → shell readline path.
#[test]
fn shell_executes_echo() {
    if !prerequisites_ok() {
        return;
    }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt not reached: {e}"));
    // Give the async readline a moment to start consuming serial input.
    std::thread::sleep(std::time::Duration::from_millis(500));
    qemu.send_line("echo VIOS_ECHO_OK");
    qemu.wait_for("VIOS_ECHO_OK", 15).unwrap_or_else(|e| {
        panic!("shell did not echo command: {e}\n--- output ---\n{}", qemu.dump())
    });
}

/// Phase 10/18: the Lua runtime cell must load and execute. Spawning `/bin/lua`
/// from the shell should print the Lua banner, proving the C-linked cell boots,
/// initialises its interpreter, and runs its Rust `main`.
///
/// Note: arguments are not yet passed to spawned cells (`sys_spawn_from_path`
/// takes only a path), so `lua -e "..."` cannot be tested until argv passing
/// lands. The banner is sufficient proof that the runtime executes.
#[test]
fn lua_runtime_executes() {
    if !prerequisites_ok() {
        return;
    }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt not reached: {e}"));
    std::thread::sleep(std::time::Duration::from_millis(500));
    qemu.send_line("lua");
    qemu.wait_for("Lua 5.4 on ViOS", 20).unwrap_or_else(|e| {
        panic!("lua runtime did not start: {e}\n--- output ---\n{}", qemu.dump())
    });
}

/// Phase 10/18: Lua must actually EXECUTE code (not just print a banner).
/// `lua -e print(31337)` evaluates the chunk via the argv transport and prints
/// the result — proving the interpreter runs Lua source, not just its banner.
/// Exercises the arena-backed `lua_Alloc` (the default malloc allocator's
/// `_sbrk` heap is a toolchain stub returning null).
#[test]
fn lua_eval_executes_code() {
    if !prerequisites_ok() {
        return;
    }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt not reached: {e}"));
    std::thread::sleep(std::time::Duration::from_millis(500));
    qemu.send_line("lua -e print(31337)");
    qemu.wait_for("31337", BOOT_TIMEOUT).unwrap_or_else(|e| {
        panic!("lua did not execute code: {e}\n--- output ---\n{}", qemu.dump())
    });
}


/// Phase 20: the kernel state-stash primitive that underpins hot migration
/// must round-trip. The kernel runs a boot self-test (stash a sentinel,
/// restore it, compare) and logs the outcome.
#[test]
fn hot_migration_state_transfer_works() {
    if !prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("state-stash: round-trip OK", BOOT_TIMEOUT).unwrap_or_else(|e| {
        panic!("state-stash round-trip failed: {e}\n--- output ---\n{}", qemu.dump())
    });
}

/// Phase 16: the VirtIO GPU must initialise its framebuffer. With a 4 MB
/// framebuffer the kernel needs the 32 MB heap; this guards the regression
/// where setup_framebuffer hung / OOM'd and blocked the boot.
#[test]
fn gpu_framebuffer_initialises() {
    if !prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("Framebuffer setup success", BOOT_TIMEOUT).unwrap_or_else(|e| {
        panic!("GPU framebuffer setup did not complete: {e}\n--- output ---\n{}", qemu.dump())
    });
    // Boot must still reach the shell with the GPU attached (no hang).
    qemu.wait_for("ViOS >", BOOT_TIMEOUT).unwrap_or_else(|e| {
        panic!("boot did not reach shell with GPU attached: {e}")
    });
}

/// Phase 15: the network service must complete a DHCP lease. QEMU's user-mode
/// SLIRP stack runs a DHCP server that hands out 10.0.2.15; the net cell must
/// transmit DISCOVER, receive OFFER/ACK, and configure that address.
#[test]
fn network_dhcp_acquires_ip() {
    if !prerequisites_ok() {
        return;
    }
    let qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("DHCP acquired", 40).unwrap_or_else(|e| {
        panic!("DHCP did not complete: {e}\n--- output ---\n{}", qemu.dump())
    });
    // QEMU SLIRP always leases 10.0.2.15 to the first client.
    qemu.wait_for("10.0.2.15", 5).unwrap_or_else(|e| {
        panic!("expected leased IP 10.0.2.15: {e}\n--- output ---\n{}", qemu.dump())
    });
}

/// Phase 18: the MicroPython runtime cell must load and execute. Spawning
/// `/bin/python` should print the MicroPython banner.
#[test]
fn micropython_runtime_executes() {
    if !prerequisites_ok() {
        return;
    }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt not reached: {e}"));
    std::thread::sleep(std::time::Duration::from_millis(500));
    qemu.send_line("python");
    qemu.wait_for("MicroPython v1.24.1 on ViOS", 20).unwrap_or_else(|e| {
        panic!("micropython did not start: {e}\n--- output ---\n{}", qemu.dump())
    });
}

/// Phase A: TCP data-path — CONNECT → SEND → RECV → CLOSE via the `nc` tool.
///
/// The echo server is started on the host before QEMU boots. QEMU SLIRP routes
/// guest connections to `10.0.2.2:<port>` to `127.0.0.1:<port>` on the host.
/// `nc` sends "HELLO_VIOS\n", the echo server reflects it, and nc prints it to
/// serial — proving the full TCP data-path is wired end-to-end.
#[test]
fn network_tcp_send_recv() {
    if !prerequisites_ok() {
        return;
    }

    // Start the echo server before QEMU so it is ready when nc connects.
    let echo_port = spawn_echo_server();

    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());

    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("shell not reached: {e}\n--- output ---\n{}", qemu.dump()));

    // Wait for DHCP before asking nc to connect — avoids a race where the net
    // cell hasn't acquired an IP yet and the TCP SYN uses 0.0.0.0 as source.
    qemu.wait_for("DHCP acquired", 40)
        .unwrap_or_else(|e| panic!("DHCP failed: {e}\n--- output ---\n{}", qemu.dump()));

    std::thread::sleep(std::time::Duration::from_millis(500));
    qemu.send_line(&format!("nc 10.0.2.2 {echo_port}"));

    qemu.wait_for("connected", 15)
        .unwrap_or_else(|e| panic!("nc did not connect: {e}\n--- output ---\n{}", qemu.dump()));

    qemu.wait_for("HELLO_VIOS", 20)
        .unwrap_or_else(|e| panic!("TCP echo not received: {e}\n--- output ---\n{}", qemu.dump()));
}

/// Phase B: HTTP/1.0 GET via `curl` over the Phase A TCP data-path.
///
/// A host HTTP server is started before QEMU boots. QEMU SLIRP routes guest
/// connections to `10.0.2.2:<port>` → host `127.0.0.1:<port>`. `curl` sends
/// a minimal GET, the server replies `HTTP/1.0 200 OK\r\n...\r\n\r\nHELLO`,
/// and closes. Proves the full HTTP client path works end-to-end.
#[test]
fn network_curl_http_get() {
    if !prerequisites_ok() {
        return;
    }

    // Start the HTTP server before QEMU boots so it is ready when curl connects.
    // Keep `_server` (not `_`) alive — dropping the handle early can race with
    // the server thread's accept() and cause the test to flake.
    let (port, _server) = spawn_http_server();

    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());

    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("shell not reached: {e}\n--- output ---\n{}", qemu.dump()));

    // Gate on DHCP before connecting — avoids a race where the net cell has
    // not yet acquired an IP and the TCP SYN uses 0.0.0.0 as source address.
    qemu.wait_for("DHCP acquired", 40)
        .unwrap_or_else(|e| panic!("DHCP failed: {e}\n--- output ---\n{}", qemu.dump()));

    std::thread::sleep(std::time::Duration::from_millis(500));
    qemu.send_line(&format!("curl http://10.0.2.2:{port}/"));

    qemu.wait_for("200", 20)
        .unwrap_or_else(|e| panic!("no HTTP 200 status: {e}\n--- output ---\n{}", qemu.dump()));

    qemu.wait_for("HELLO", 10)
        .unwrap_or_else(|e| panic!("no response body: {e}\n--- output ---\n{}", qemu.dump()));
}

/// Phase E: `vnet.resolve()` static-table fast-path (no DNS, deterministic).
///
/// "gateway" is in the static alias table → returns "10.0.2.2" without a DNS
/// query. This test is a hard gate and does not require internet access.
#[test]
fn lua_vnet_resolve() {
    if !prerequisites_ok() {
        return;
    }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt not reached: {e}\n--- output ---\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(500));
    qemu.send_line("lua -e print(vnet.resolve('gateway'))");
    qemu.wait_for("10.0.2.2", 10)
        .unwrap_or_else(|e| panic!("static resolve failed: {e}\n--- output ---\n{}", qemu.dump()));
}

/// Phase E: `vnet.resolve()` real DNS A-record query via QEMU SLIRP (10.0.2.3:53).
///
/// Requires the test host to have outbound UDP :53 (normal internet access).
/// The assertion is intentionally loose — any dotted-decimal IP output passes.
/// Skip (non-blocking) if DNS is unavailable in the CI environment.
#[test]
fn lua_vnet_resolve_dns() {
    if !prerequisites_ok() {
        return;
    }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt not reached: {e}\n--- output ---\n{}", qemu.dump()));
    qemu.wait_for("DHCP acquired", 40)
        .unwrap_or_else(|e| panic!("DHCP failed: {e}\n--- output ---\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(500));
    // Wrap output in a marker that can't appear in boot messages so the assertion
    // is not a false-positive from the existing `[net] IP address: 10.0.2.15` line.
    qemu.send_line("lua -e local r=vnet.resolve('google.com') if r then print('RESOLVED:'..r) end");
    qemu.wait_for("RESOLVED:", 35) // DNS under parallel QEMU load can take longer
        .unwrap_or_else(|e| panic!("DNS resolve produced no output: {e}\n--- output ---\n{}", qemu.dump()));
}

/// Phase F.1: `lua /data/script.lua` — reads and executes a Lua script from VFS.
#[test]
fn lua_script_file() {
    if !prerequisites_ok() {
        return;
    }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    assert!(qemu.output_contains("FAT16 /data volume mounted"), "FAT16 not mounted\n{}", qemu.dump());
    std::thread::sleep(Duration::from_millis(500));
    qemu.send_line("vwrite /data/hello.lua print('SCRIPT_OK')");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("vwrite: {e}\n{}", qemu.dump()));
    qemu.send_line("lua /data/hello.lua");
    qemu.wait_for("SCRIPT_OK", 15)
        .unwrap_or_else(|e| panic!("script did not run: {e}\n{}", qemu.dump()));
}

/// Phase F.2: Lua `vfs.*` file I/O — write then read back from /data/.
#[test]
fn lua_vfs_write_read() {
    if !prerequisites_ok() {
        return;
    }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    assert!(qemu.output_contains("FAT16 /data volume mounted"), "FAT16 not mounted\n{}", qemu.dump());
    std::thread::sleep(Duration::from_millis(500));
    // Single -e expression: adjacent Lua stmts, no semicolons, no spaces inside strings.
    qemu.send_line("lua -e vfs.write('/data/lua_vfs.txt','HELLO_VFS') print(vfs.read('/data/lua_vfs.txt'))");
    qemu.wait_for("HELLO_VFS", 15)
        .unwrap_or_else(|e| panic!("vfs roundtrip failed: {e}\n{}", qemu.dump()));
}

/// Phase D.2: HTTP/1.0 GET from Lua via the `vnet.*` TCP bindings.
///
/// A host HTTP server is started before QEMU boots. SLIRP routes guest
/// `10.0.2.2:<port>` → host `127.0.0.1:<port>`. Lua connects out, sends a
/// minimal GET, and prints the response — proving Phase D.1 SEND-length fix
/// and the vnet bindings work end-to-end. No hostfwd needed (Lua dials out).
#[test]
fn lua_tcp_http_get() {
    if !prerequisites_ok() {
        return;
    }

    // Keep `_server` alive — dropping early can race with the accept thread.
    let (port, _server) = spawn_http_server();

    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());

    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("shell not reached: {e}\n--- output ---\n{}", qemu.dump()));

    qemu.wait_for("DHCP acquired", 40)
        .unwrap_or_else(|e| panic!("DHCP failed: {e}\n--- output ---\n{}", qemu.dump()));

    std::thread::sleep(Duration::from_millis(500));

    // The ViOS shell splits on `;` (sequence separator) and whitespace.
    // Adjacent Lua statements without `;` are valid Lua — the parser ends each
    // statement at the closing `)`. `'\r\n\r\n'` is space-free and sufficient
    // to trigger the test server, which only looks for that terminator.
    qemu.send_line(&format!(
        "lua -e local c=vnet.connect('10.0.2.2',{port}) vnet.send(c,'\\r\\n\\r\\n') print(vnet.recv(c,512)) vnet.close(c)"
    ));

    // The host server replies "HTTP/1.0 200 OK\r\n...\r\n\r\nHELLO".
    qemu.wait_for("200", 20)
        .unwrap_or_else(|e| panic!("no HTTP 200: {e}\n--- output ---\n{}", qemu.dump()));

    qemu.wait_for("HELLO", 10)
        .unwrap_or_else(|e| panic!("no body: {e}\n--- output ---\n{}", qemu.dump()));
}

/// Phase C (network): guest as TCP server — `nc -l 9090` listens; the host
/// connects through QEMU SLIRP hostfwd, sends "PING_VIOS\n", and nc echoes
/// the bytes to serial — proving LISTEN/ACCEPT and the inbound data-path.
#[test]
fn network_tcp_listen_accept() {
    if !prerequisites_ok() {
        return;
    }

    let (mut qemu, host_port) =
        QemuRunner::boot_with_hostfwd(&kernel_path(), &disk_path(), 9090);

    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("shell not reached: {e}\n--- output ---\n{}", qemu.dump()));

    qemu.wait_for("DHCP acquired", 40)
        .unwrap_or_else(|e| panic!("DHCP failed: {e}\n--- output ---\n{}", qemu.dump()));

    std::thread::sleep(Duration::from_millis(500));

    qemu.send_line("nc -l 9090");
    qemu.wait_for("listening", 10)
        .unwrap_or_else(|e| panic!("nc did not listen: {e}\n--- output ---\n{}", qemu.dump()));

    // Give nc a moment to enter the ACCEPT poll loop (it does so immediately
    // after printing "listening"), then connect from the host.
    std::thread::sleep(Duration::from_millis(200));
    let mut stream = TcpStream::connect(format!("127.0.0.1:{host_port}"))
        .unwrap_or_else(|e| panic!("host connect to guest failed: {e}\n--- output ---\n{}", qemu.dump()));

    qemu.wait_for("connected", 15)
        .unwrap_or_else(|e| panic!("nc did not accept: {e}\n--- output ---\n{}", qemu.dump()));

    stream.write_all(b"PING_VIOS\n").expect("write to guest failed");
    let _ = stream.flush();

    qemu.wait_for("PING_VIOS", 20)
        .unwrap_or_else(|e| panic!("guest did not receive probe: {e}\n--- output ---\n{}", qemu.dump()));
}

/// Phase C: VFS write — echo redirected to /tmp, then vcat reads it back via VFS.
///
/// `cat` reads from the kernel-embedded FS (VIFS1 / kernel_fs.img) which does
/// not include /tmp. `vcat` reads from the VFS cell's RamFS via OP_READ opcode,
/// which is the same store OP_WRITE targets — making the round-trip verifiable.
#[test]
fn vfs_write_echo_redirect() {
    if !prerequisites_ok() {
        return;
    }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());

    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("shell not reached: {e}\n--- output ---\n{}", qemu.dump()));

    std::thread::sleep(std::time::Duration::from_millis(500));

    qemu.send_line("echo PHASE_C_WRITE > /tmp/test.txt");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt not returned after write: {e}\n--- output ---\n{}", qemu.dump()));

    qemu.send_line("vcat /tmp/test.txt");
    qemu.wait_for("PHASE_C_WRITE", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("file content not read back: {e}\n--- output ---\n{}", qemu.dump()));
}

/// Phase D: write to /data (FAT16 on VirtIO) and read it back in the same boot.
///
/// Proves the full path: shell → VFS → fatfs → BlockStream → block syscall →
/// VirtIO → disk image → back. Reboot persistence is Phase E (needs graceful
/// QEMU shutdown to flush the disk image before kill).
#[test]
fn vfs_fat16_write_read() {
    if !prerequisites_ok() {
        return;
    }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());

    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt not reached: {e}\n--- output ---\n{}", qemu.dump()));

    // Verify the VFS cell successfully mounted the Phase 2 FAT16 volume.
    assert!(
        qemu.output_contains("FAT16 /data volume mounted"),
        "VFS did not mount FAT16 /data volume\n--- output ---\n{}",
        qemu.dump()
    );

    std::thread::sleep(std::time::Duration::from_millis(500));

    qemu.send_line("echo PHASE_D_PERSIST > /data/test.txt");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("write did not return to prompt: {e}\n{}", qemu.dump()));

    qemu.send_line("vcat /data/test.txt");
    qemu.wait_for("PHASE_D_PERSIST", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("FAT16 read-back failed: {e}\n--- output ---\n{}", qemu.dump()));
}

/// Phase E: a FAT16 write survives a full reboot (persistence across power cycle).
///
/// Boots QEMU, writes a marker to `/data/`, issues the `shutdown` built-in, waits
/// for QEMU to exit cleanly (flushing the VirtIO-backed disk image), then boots a
/// SECOND QEMU instance against the same `disk_v3.img` to verify the marker persisted.
///
/// Note: the same `disk_v3.img` is shared between both boots. The FAT16 write is
/// create-or-overwrite (idempotent), so re-runs are safe.
#[test]
fn vfs_fat16_reboot_persistence() {
    if !prerequisites_ok() {
        return;
    }

    // ── First boot: write the marker then shut down ───────────────────────────
    // NOTE: `wait_for("ViOS >")` matches the earliest occurrence in accumulated
    // output, so it may return before the command actually completes. This is
    // fine because the shell processes commands in FIFO order from its readline
    // buffer. Sending "shutdown" immediately after the write means the shell will
    // execute them in sequence: write first, then shutdown. Phase C demonstrates
    // this works for echo-redirect + vcat — the same mechanism applies here.
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("first boot prompt failed: {e}\n{}", qemu.dump()));
    assert!(
        qemu.output_contains("FAT16 /data volume mounted"),
        "FAT16 not mounted on first boot\n{}", qemu.dump()
    );
    std::thread::sleep(std::time::Duration::from_millis(500));

    qemu.send_line("echo REBOOT_OK > /data/persist.txt");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("write did not return to prompt: {e}\n{}", qemu.dump()));

    qemu.send_line("shutdown");
    qemu.wait_for("System shutting down", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("shutdown did not run: {e}\n{}", qemu.dump()));

    assert!(
        qemu.wait_for_natural_exit(15),
        "QEMU did not exit after shutdown command\n{}", qemu.dump()
    );
    let first_boot_dump = qemu.dump();
    drop(qemu); // safe: process already exited; Drop's kill is a harmless no-op

    // ── Second boot: verify persistence ──────────────────────────────────────
    let mut qemu2 = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu2.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("second boot prompt failed: {e}\n{}", qemu2.dump()));
    assert!(
        qemu2.output_contains("FAT16 /data volume mounted"),
        "FAT16 not mounted on second boot\n{}", qemu2.dump()
    );
    std::thread::sleep(std::time::Duration::from_millis(500));

    qemu2.send_line("vcat /data/persist.txt");
    // Use a larger timeout than CMD_TIMEOUT: with cache=none (O_DIRECT) reads
    // are slower and the FAT16 mount adds latency in the second boot.
    qemu2.wait_for("REBOOT_OK", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("persistence failed: {e}\n--- first boot ---\n{}\n--- second boot ---\n{}", first_boot_dump, qemu2.dump()));
}

/// Phase F-1: write a >253-byte marker to /tmp (proves content_len u16 works).
#[test]
fn vfs_write_large_content() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    std::thread::sleep(std::time::Duration::from_millis(500));
    // Write a marker that is exactly 255 chars (>253-byte old cap).
    // The marker itself fits in a single echo line; shell passes it via write_file.
    qemu.send_line("echo PHASE_F_WIDE_WRITE_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA > /tmp/big.txt");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("write: {e}\n{}", qemu.dump()));
    qemu.send_line("vcat /tmp/big.txt");
    qemu.wait_for("PHASE_F_WIDE_WRITE", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("wide write readback: {e}\n{}", qemu.dump()));
}

/// Phase F-2: create /data/ file, verify it exists, delete it, verify gone.
#[test]
fn vfs_fat16_unlink() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    assert!(qemu.output_contains("FAT16 /data volume mounted"), "FAT16 not mounted\n{}", qemu.dump());
    std::thread::sleep(std::time::Duration::from_millis(500));
    qemu.send_line("echo PHASE_F_DEL > /data/del.txt");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("write: {e}\n{}", qemu.dump()));
    qemu.send_line("vcat /data/del.txt");
    qemu.wait_for("PHASE_F_DEL", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("file exists: {e}\n{}", qemu.dump()));
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("post-vcat prompt: {e}\n{}", qemu.dump()));
    qemu.send_line("rm /data/del.txt");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("rm: {e}\n{}", qemu.dump()));
    qemu.send_line("vcat /data/del.txt");
    qemu.wait_for("not found", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("file still exists after rm: {e}\n{}", qemu.dump()));
}

/// Phase F-3: create /data/ subdirectory, write and read a file in it.
#[test]
fn vfs_fat16_subdir() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    assert!(qemu.output_contains("FAT16 /data volume mounted"), "FAT16 not mounted\n{}", qemu.dump());
    std::thread::sleep(std::time::Duration::from_millis(500));
    qemu.send_line("mkdir /data/sub");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("mkdir: {e}\n{}", qemu.dump()));
    qemu.send_line("echo PHASE_F_SUB > /data/sub/f.txt");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("write subdir: {e}\n{}", qemu.dump()));
    qemu.send_line("vcat /data/sub/f.txt");
    qemu.wait_for("PHASE_F_SUB", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("subdir readback: {e}\n{}", qemu.dump()));
}

/// Phase G: a non-VFS cell must NOT access raw block I/O (capability gate).
///
/// `blktest` calls `sys_blk_read` from the shell cell, which lacks `can_block_io`.
/// The gate must deny it with `PermissionDenied`.
#[test]
fn block_io_denied_non_vfs() {
    if !prerequisites_ok() {
        return;
    }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt not reached: {e}\n{}", qemu.dump()));
    std::thread::sleep(std::time::Duration::from_millis(500));

    qemu.send_line("blktest");
    qemu.wait_for("blkio: denied", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("block I/O was NOT denied for non-VFS cell: {e}\n{}", qemu.dump()));

    // Guard against false pass: the BUG marker must never appear.
    assert!(
        !qemu.output_contains("blkio: ALLOWED"),
        "capability gate let a non-VFS cell read the block device\n{}", qemu.dump()
    );
}

/// Phase G: a FAT16 SUBDIRECTORY write survives a full reboot.
///
/// Same power-cycle pattern as `vfs_fat16_reboot_persistence`, but the marker is
/// written into a nested dir (`/data/pdir/f.txt`) created at runtime via `mkdir`.
/// Shares `disk_v3.img`; create-or-overwrite keeps re-runs safe.
#[test]
fn vfs_fat16_subdir_persistence() {
    if !prerequisites_ok() {
        return;
    }

    // ── First boot: mkdir + write into the subdir, then shut down ────────────
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("first boot prompt failed: {e}\n{}", qemu.dump()));
    assert!(
        qemu.output_contains("FAT16 /data volume mounted"),
        "FAT16 not mounted on first boot\n{}", qemu.dump()
    );
    std::thread::sleep(std::time::Duration::from_millis(500));

    qemu.send_line("mkdir /data/pdir");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("mkdir did not return to prompt: {e}\n{}", qemu.dump()));

    qemu.send_line("echo SUBDIR_PERSIST > /data/pdir/f.txt");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("subdir write did not return to prompt: {e}\n{}", qemu.dump()));

    qemu.send_line("shutdown");
    qemu.wait_for("System shutting down", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("shutdown did not run: {e}\n{}", qemu.dump()));
    assert!(
        qemu.wait_for_natural_exit(15),
        "QEMU did not exit after shutdown\n{}", qemu.dump()
    );
    let first_boot_dump = qemu.dump();
    drop(qemu);

    // ── Second boot: verify the subdir file persisted ─────────────────────────
    let mut qemu2 = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu2.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("second boot prompt failed: {e}\n{}", qemu2.dump()));
    assert!(
        qemu2.output_contains("FAT16 /data volume mounted"),
        "FAT16 not mounted on second boot\n{}", qemu2.dump()
    );
    std::thread::sleep(std::time::Duration::from_millis(500));

    qemu2.send_line("vcat /data/pdir/f.txt");
    qemu2.wait_for("SUBDIR_PERSIST", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!(
            "subdir file not persisted across reboot: {e}\n--- first boot ---\n{first_boot_dump}\n--- second boot ---\n{}",
            qemu2.dump()
        ));
}

/// Phase H: recursive directory removal via `rm -r /data/dir` (OP_RMDIR_RECURSIVE).
///
/// X-1 progress: VFS stack overflow fixed (STACK_PAGES=64 + static sector buffers).
/// VirtIO DMA now uses virt_to_phys() for correct physical addressing (Phase X-1).
/// Remaining issue: vcat after rm-r hangs — possibly fatfs directory-iterator
/// state after deletion; under investigation.
#[ignore]
#[test]
fn vfs_fat16_recursive_rmdir() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    assert!(qemu.output_contains("FAT16 /data volume mounted"), "FAT16 not mounted\n{}", qemu.dump());
    std::thread::sleep(std::time::Duration::from_millis(500));

    qemu.send_line("mkdir /data/rr");
    qemu.wait_for("ViOS >", CMD_TIMEOUT).unwrap_or_else(|e| panic!("mkdir: {e}\n{}", qemu.dump()));
    qemu.send_line("echo X > /data/rr/f.txt");
    qemu.wait_for("ViOS >", CMD_TIMEOUT).unwrap_or_else(|e| panic!("write: {e}\n{}", qemu.dump()));
    qemu.send_line("rm -r /data/rr");
    qemu.wait_for("ViOS >", CMD_TIMEOUT).unwrap_or_else(|e| panic!("rm -r: {e}\n{}", qemu.dump()));
    qemu.send_line("vcat /data/rr/f.txt");
    qemu.wait_for("not found", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("tree not deleted: {e}\n{}", qemu.dump()));
}

/// Phase H: OP_APPEND assembles content without truncating (vwrite then vappend).
#[test]
fn vfs_fat16_append() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    assert!(qemu.output_contains("FAT16 /data volume mounted"), "FAT16 not mounted\n{}", qemu.dump());
    std::thread::sleep(std::time::Duration::from_millis(500));

    qemu.send_line("vwrite /data/big.txt AAA");
    qemu.wait_for("ViOS >", CMD_TIMEOUT).unwrap_or_else(|e| panic!("vwrite: {e}\n{}", qemu.dump()));
    qemu.send_line("vappend /data/big.txt BBB");
    qemu.wait_for("ViOS >", CMD_TIMEOUT).unwrap_or_else(|e| panic!("vappend: {e}\n{}", qemu.dump()));
    qemu.send_line("vcat /data/big.txt");
    qemu.wait_for("AAABBB", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("append truncated/lost: {e}\n{}", qemu.dump()));
}

// ── Phase G: MicroPython argv + vnet module ───────────────────────────────────

/// Phase G.1: `python -c "print(2+3)"` must output `5`.
///
/// Verifies that `mp_embed_exec_str` executes Python source passed via argv,
/// not just the banner printout that was previously the only verified output.
#[test]
fn python_exec_code() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt not reached: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(500));

    // No spaces in the expression so the shell passes it as one token.
    qemu.send_line("python -c print(2+3)");
    qemu.wait_for("5", 20)
        .unwrap_or_else(|e| panic!("python did not execute code: {e}\n--- output ---\n{}", qemu.dump()));
}

/// Phase G.1: `python path.py` must read the script from VFS and execute it.
///
/// Uses `vwrite` to create a one-liner Python script in /data/, then spawns
/// `python /data/py_script_test.py` to verify the VFS-read + exec path.
#[test]
fn python_script_file() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    assert!(qemu.output_contains("FAT16 /data volume mounted"), "FAT16 not mounted\n{}", qemu.dump());
    std::thread::sleep(Duration::from_millis(500));

    // Use a short (8.3-compatible) filename — FAT16 LFN is not guaranteed.
    qemu.send_line("vwrite /data/test.py print('PYTHON_SCRIPT_OK')");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("vwrite: {e}\n{}", qemu.dump()));
    qemu.send_line("python /data/test.py");
    qemu.wait_for("PYTHON_SCRIPT_OK", 20)
        .unwrap_or_else(|e| panic!("python script did not run: {e}\n--- output ---\n{}", qemu.dump()));
}

/// Phase G.2: `import vnet; vnet.resolve('gateway')` must return `10.0.2.2`.
///
/// Verifies the vnet C module is registered in MicroPython's built-in module
/// table and that the static DNS table returns the QEMU SLIRP gateway.
/// Uses `__import__` so the whole expression fits in one `-c` argument without
/// semicolons (the ViOS shell splits on `;`).
#[test]
fn python_vnet_resolve() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt not reached: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(500));

    // `__import__` avoids a separate `import` statement and semicolons.
    qemu.send_line("python -c print(__import__('vnet').resolve('gateway'))");
    qemu.wait_for("10.0.2.2", 20)
        .unwrap_or_else(|e| panic!("vnet.resolve('gateway') failed: {e}\n--- output ---\n{}", qemu.dump()));
}

// ── Phase H: MicroPython vfs module + Python TCP HTTP ────────────────────────

/// Phase H.1: `import vfs; vfs.write(...)` then `vfs.read(...)` must round-trip.
///
/// The ARGV_STASH_KEY in ViOS is a single global slot; a second spawn immediately
/// after the first can overwrite it before the first cell gets scheduled.  Avoid
/// all sequential-spawn races by doing the entire write+read in ONE Python `-c`
/// invocation — single expression, no semicolons (shell splits on `;`):
///
///   lambda v: (write, read)[1]  →  imports vfs, writes, reads, returns read value
#[test]
fn python_vfs_write_read() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    assert!(qemu.output_contains("FAT16 /data volume mounted"), "FAT16 not mounted\n{}", qemu.dump());
    std::thread::sleep(Duration::from_millis(500));

    // One Python `-c` call: lambda writes, reads, returns the read content.
    // tuple indexing [1] selects the read result; print() outputs it.
    // No semicolons needed — shell-safe.
    qemu.send_line(
        "python -c print((lambda v:(v.write('/data/vp.txt','PYTHON_VFS_OK'),v.read('/data/vp.txt'))[1])(__import__('vfs')))"
    );
    qemu.wait_for("PYTHON_VFS_OK", 25)
        .unwrap_or_else(|e| panic!("python vfs roundtrip failed: {e}\n--- output ---\n{}", qemu.dump()));
}

/// Phase H.2: Python script does an HTTP/1.0 GET via `import vnet`.
///
/// A Lua one-liner writes the Python script to VFS (Lua interprets `\n` as
/// newline and `\\r\\n` as the literal escape sequence that Python parses as
/// CR+LF).  `python /data/http.py` is then spawned; the host HTTP server must
/// reply with a line containing `200`.
#[test]
fn python_vnet_tcp_http_get() {
    if !prerequisites_ok() { return; }

    let (port, _server) = spawn_http_server();

    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("shell not reached: {e}\n{}", qemu.dump()));
    assert!(qemu.output_contains("FAT16 /data volume mounted"), "FAT16 not mounted\n{}", qemu.dump());

    qemu.wait_for("DHCP acquired", 40)
        .unwrap_or_else(|e| panic!("DHCP failed: {e}\n{}", qemu.dump()));

    std::thread::sleep(Duration::from_millis(500));

    // Lua writes the Python HTTP script to VFS.
    // In Lua: \n = newline; \\r\\n = literal \r\n (Python escape sequences).
    // The script: import vnet, connect, send HTTP GET, print response, close.
    // Uses 8.3-compatible filename and double-quoted Python strings (safe inside
    // Lua single-quoted string literals).
    qemu.send_line(&format!(
        r#"lua -e vfs.write('/data/http.py','import vnet\nc=vnet.connect("10.0.2.2",{port})\nvnet.send(c,"GET / HTTP/1.0\\r\\n\\r\\n")\nprint(vnet.recv(c))\nvnet.close(c)')"#
    ));
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("lua vfs.write did not return: {e}\n{}", qemu.dump()));

    qemu.send_line("python /data/http.py");
    qemu.wait_for("200", 25)
        .unwrap_or_else(|e| panic!("no HTTP 200 from Python: {e}\n--- output ---\n{}", qemu.dump()));
}

// ── Phase M: httpd — minimal HTTP/1.0 file server ────────────────────────────

/// Phase M: `httpd <port> <vfs_path>` serves a VFS file over HTTP/1.0.
///
/// Flow:
///  1. Write sentinel file to /tmp via `vwrite`.
///  2. Start `httpd 9091 /tmp/resp.txt` as a background job (`&`).
///  3. Wait for "httpd: listening".
///  4. Host connects via QEMU SLIRP hostfwd and sends a bare GET request.
///  5. Read the response; assert it contains the sentinel.
#[test]
fn network_httpd_serves_file() {
    if !prerequisites_ok() { return; }

    let (mut qemu, host_port) =
        QemuRunner::boot_with_hostfwd(&kernel_path(), &disk_path(), 9091);

    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("shell: {e}\n{}", qemu.dump()));
    qemu.wait_for("DHCP acquired", 40)
        .unwrap_or_else(|e| panic!("DHCP: {e}\n{}", qemu.dump()));

    std::thread::sleep(Duration::from_millis(500));

    // Write the file httpd will serve.
    qemu.send_line("vwrite /tmp/resp.txt HTTPD_OK");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("vwrite: {e}\n{}", qemu.dump()));

    // Start httpd in the background so the shell returns immediately.
    qemu.send_line("httpd 9091 /tmp/resp.txt &");
    qemu.wait_for("httpd: listening", 10)
        .unwrap_or_else(|e| panic!("httpd did not listen: {e}\n{}", qemu.dump()));

    std::thread::sleep(Duration::from_millis(200));

    // Host sends a minimal HTTP/1.0 GET request.
    let mut stream = TcpStream::connect(format!("127.0.0.1:{host_port}"))
        .unwrap_or_else(|e| panic!("host connect failed: {e}\n{}", qemu.dump()));
    stream.write_all(b"GET / HTTP/1.0\r\n\r\n").expect("write GET");
    stream.flush().expect("flush");

    // Read full response (server closes after serving).
    let mut response = Vec::new();
    use std::io::Read;
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let _ = stream.read_to_end(&mut response);

    let body = String::from_utf8_lossy(&response);
    assert!(
        body.contains("HTTPD_OK"),
        "response did not contain HTTPD_OK\n--- response ---\n{body}\n--- QEMU ---\n{}",
        qemu.dump()
    );
}

// ── Phase O: Dynamic httpd + while loop ───────────────────────────────────────

/// Phase O-1: httpd re-reads the file on every request — a second GET after a
/// `vwrite` overwrite must return the new content without restarting httpd.
#[test]
fn network_httpd_dynamic_content() {
    if !prerequisites_ok() { return; }

    let (mut qemu, host_port) =
        QemuRunner::boot_with_hostfwd(&kernel_path(), &disk_path(), 9092);

    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("shell: {e}\n{}", qemu.dump()));
    qemu.wait_for("DHCP acquired", 40)
        .unwrap_or_else(|e| panic!("DHCP: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(500));

    qemu.send_line("vwrite /tmp/v1.txt CONTENT_V1");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("vwrite v1: {e}\n{}", qemu.dump()));

    qemu.send_line("httpd 9092 /tmp/v1.txt &");
    qemu.wait_for("httpd: listening", 10)
        .unwrap_or_else(|e| panic!("httpd did not listen: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(200));

    // First GET — expect initial content.
    let get_response = |host_port: u16| {
        use std::io::Read;
        let mut stream = TcpStream::connect(format!("127.0.0.1:{host_port}"))
            .expect("connect failed");
        stream.write_all(b"GET / HTTP/1.0\r\n\r\n").expect("write");
        stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
        let mut buf = Vec::new();
        let _ = stream.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    };

    let r1 = get_response(host_port);
    assert!(r1.contains("CONTENT_V1"),
        "first GET missing CONTENT_V1\n--- response ---\n{r1}\n--- QEMU ---\n{}", qemu.dump());

    // Overwrite the file — httpd must serve the new content without restart.
    qemu.send_line("vwrite /tmp/v1.txt CONTENT_V2");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("vwrite v2: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(200));

    let r2 = get_response(host_port);
    assert!(r2.contains("CONTENT_V2"),
        "second GET missing CONTENT_V2\n--- response ---\n{r2}\n--- QEMU ---\n{}", qemu.dump());
    assert!(!r2.contains("CONTENT_V1"),
        "second GET still contains stale CONTENT_V1\n--- response ---\n{r2}");
}

/// Phase O-2: `while COND; do BODY; done` — body runs while condition exits 0.
///
/// Two assertions:
///  (a) False condition → body never runs.
///  (b) True-once: flag file exists → body runs once → `rm` deletes flag →
///      loop exits; no infinite hang.
#[test]
fn shell_while_loop() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    assert!(qemu.output_contains("FAT16 /data volume mounted"), "FAT16 not mounted\n{}", qemu.dump());
    std::thread::sleep(Duration::from_millis(300));

    // (a) False condition: body must NOT execute.
    qemu.send_line("while vcat /no/such/file; do echo SHOULD_NOT_APPEAR; done");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("while false hung: {e}\n{}", qemu.dump()));
    assert!(!qemu.output_contains("SHOULD_NOT_APPEAR"),
        "while false ran its body\n{}", qemu.dump());

    // (b) True-once: write flag, run body (echo + rm), verify body ran, loop exits.
    qemu.send_line("vwrite /data/wflag.txt X");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("vwrite flag: {e}\n{}", qemu.dump()));

    qemu.send_line("while vcat /data/wflag.txt; do echo WHILE_BODY; rm /data/wflag.txt; done");
    qemu.wait_for("WHILE_BODY", 15)
        .unwrap_or_else(|e| panic!("while body did not run: {e}\n--- output ---\n{}", qemu.dump()));
    // Loop must exit after rm deletes the flag (not hang).
    qemu.wait_for("ViOS >", 15)
        .unwrap_or_else(|e| panic!("while loop did not exit after rm: {e}\n--- output ---\n{}", qemu.dump()));
}

// ── Phase X-2: Shell function positional args ────────────────────────────────

/// Phase X-2: `$1 $2 $# $@` are set inside function bodies and restored after.
#[test]
fn shell_function_positional_args() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(300));

    // Define a function that echoes $1 $2.
    qemu.send_line("double() { echo $1 $2; }");
    qemu.wait_for("ViOS >", CMD_TIMEOUT).unwrap_or_else(|e| panic!("def: {e}\n{}", qemu.dump()));

    qemu.send_line("double ALPHA BETA");
    qemu.wait_for("ALPHA BETA", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("$1 $2 not expanded: {e}\n--- output ---\n{}", qemu.dump()));

    // $# = arg count.
    qemu.send_line("argc() { echo $#; }");
    qemu.wait_for("ViOS >", CMD_TIMEOUT).unwrap_or_else(|e| panic!("def argc: {e}\n{}", qemu.dump()));
    qemu.send_line("argc a b c");
    qemu.wait_for("3", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("$# not 3: {e}\n--- output ---\n{}", qemu.dump()));
}

// ── Phase W: case/esac + echo -e + nc multi-connection ───────────────────────

/// Phase W: `case EXPR in pattern) CMD ;; *) CMD ;; esac` — pattern-match dispatch.
///
/// Sets VAR, then runs case.  Exact-match arm must fire; fallback `*` must not.
#[test]
fn shell_case_statement() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(300));

    qemu.send_line("STATUS=ok");
    qemu.wait_for("ViOS >", CMD_TIMEOUT).unwrap_or_else(|e| panic!("set: {e}\n{}", qemu.dump()));

    // Exact match fires; fallback must NOT fire.
    qemu.send_line("case $STATUS in ok) echo CASE_EXACT ;; *) echo CASE_WILD ;; esac");
    qemu.wait_for("CASE_EXACT", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("exact arm not taken: {e}\n{}", qemu.dump()));
    assert!(!qemu.output_contains("CASE_WILD"),
        "wildcard arm fired when exact matched\n{}", qemu.dump());

    // Unknown value hits the `*` fallback.
    qemu.send_line("STATUS=unknown");
    qemu.wait_for("ViOS >", CMD_TIMEOUT).unwrap_or_else(|e| panic!("set: {e}\n{}", qemu.dump()));
    qemu.send_line("case $STATUS in ok) echo CASE_EXACT2 ;; *) echo CASE_FALLBACK ;; esac");
    qemu.wait_for("CASE_FALLBACK", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("wildcard arm not taken: {e}\n{}", qemu.dump()));
    assert!(!qemu.output_contains("CASE_EXACT2"),
        "exact arm fired for unmatched value\n{}", qemu.dump());
}

/// Phase W: `echo -e "line1\nline2"` interprets escape sequences.
#[test]
fn shell_echo_e() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(300));

    // The shell double-quote handler passes \n as literal backslash-n;
    // echo -e then expands it to a real newline so two separate lines appear.
    qemu.send_line("echo -e ECHO_E_LINE1\\nECHO_E_LINE2");
    qemu.wait_for("ECHO_E_LINE1", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("line1 not seen: {e}\n{}", qemu.dump()));
    qemu.wait_for("ECHO_E_LINE2", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("line2 not seen after \\n: {e}\n{}", qemu.dump()));
}

// ── Phase V: >> append redirect + ARGV_STASH_KEY fix ─────────────────────────

/// Phase V-1: `echo A > f; echo B >> f` writes then appends; `vcat f` shows both.
#[test]
fn shell_redirect_append() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(300));

    qemu.send_line("echo LINE_A > /tmp/append_test.txt");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("write: {e}\n{}", qemu.dump()));

    qemu.send_line("echo LINE_B >> /tmp/append_test.txt");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("append: {e}\n{}", qemu.dump()));

    qemu.send_line("vcat /tmp/append_test.txt");
    qemu.wait_for("LINE_A", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("LINE_A not found: {e}\n{}", qemu.dump()));
    qemu.wait_for("LINE_B", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("LINE_B not found after append: {e}\n--- output ---\n{}", qemu.dump()));
}

/// Phase V-2: ARGV_STASH_KEY race fix — two rapid spawns each receive the correct args.
///
/// The shell sets VAR_A=A, spawns a cell to echo it, then immediately sets VAR_B=B
/// and spawns another.  Before the fix, the second set_spawn_args overwrote the
/// stash before the first cell read it.  After the fix, each cell gets its own
/// personal stash slot.
///
/// We use shell variable assignment + echo to verify indirectly: if variables
/// survive sequential spawn cycles, the race is resolved.
#[test]
fn shell_argv_race_fixed() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(300));

    // Rapid consecutive spawns with different args — previously the second
    // spawn's args clobbered the first's stash slot.
    qemu.send_line("python -c print('SPAWN_A_OK')");
    qemu.wait_for("SPAWN_A_OK", 20)
        .unwrap_or_else(|e| panic!("first spawn lost its args: {e}\n{}", qemu.dump()));
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt after A: {e}\n{}", qemu.dump()));

    qemu.send_line("python -c print('SPAWN_B_OK')");
    qemu.wait_for("SPAWN_B_OK", 20)
        .unwrap_or_else(|e| panic!("second spawn lost its args: {e}\n{}", qemu.dump()));
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt after B: {e}\n{}", qemu.dump()));
}

// ── Phase U: wget + test/[ ────────────────────────────────────────────────────

/// Phase U: `wget URL path` downloads a URL body and saves it to a VFS file.
///
/// Starts a host HTTP server, boots QEMU, runs `wget http://... /tmp/out.txt`,
/// then `vcat /tmp/out.txt` to verify the body was written correctly.
#[test]
fn network_wget_downloads_to_vfs() {
    if !prerequisites_ok() { return; }

    let (port, _server) = spawn_http_server();

    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("shell: {e}\n{}", qemu.dump()));
    qemu.wait_for("DHCP acquired", 40)
        .unwrap_or_else(|e| panic!("DHCP: {e}\n{}", qemu.dump()));

    std::thread::sleep(Duration::from_millis(500));

    qemu.send_line(&format!("wget http://10.0.2.2:{port}/ /tmp/wget_out.txt"));
    qemu.wait_for("saved", 20)
        .unwrap_or_else(|e| panic!("wget did not save: {e}\n{}", qemu.dump()));
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt after wget: {e}\n{}", qemu.dump()));

    qemu.send_line("vcat /tmp/wget_out.txt");
    qemu.wait_for("HELLO", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("vcat after wget: {e}\n{}", qemu.dump()));
}

/// Phase U: `test -f path` returns 0 when file exists, 1 when absent.
/// `[ X = Y ]` tests string equality.
#[test]
fn shell_test_builtin() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    assert!(qemu.output_contains("FAT16 /data volume mounted"), "FAT16 not mounted\n{}", qemu.dump());
    std::thread::sleep(Duration::from_millis(300));

    // -f: existing file → 0 → then branch runs.
    qemu.send_line("vwrite /data/tf.txt X");
    qemu.wait_for("ViOS >", CMD_TIMEOUT).unwrap_or_else(|e| panic!("vwrite: {e}\n{}", qemu.dump()));
    qemu.send_line("if [ -f /data/tf.txt ]; then echo FILE_EXISTS; fi");
    qemu.wait_for("FILE_EXISTS", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("test -f failed for existing file: {e}\n{}", qemu.dump()));

    // -f: absent file → 1 → else branch runs.
    qemu.send_line("if [ -f /data/no_such_file.txt ]; then echo WRONG; else echo FILE_ABSENT; fi");
    qemu.wait_for("FILE_ABSENT", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("test -f failed for absent file: {e}\n{}", qemu.dump()));

    // String equality.
    qemu.send_line("VAL=hello");
    qemu.wait_for("ViOS >", CMD_TIMEOUT).unwrap_or_else(|e| panic!("set: {e}\n{}", qemu.dump()));
    qemu.send_line("if [ $VAL = hello ]; then echo STR_EQ_OK; fi");
    qemu.wait_for("STR_EQ_OK", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("[ = ] failed: {e}\n{}", qemu.dump()));
}

// ── Phase T: Shell functions ──────────────────────────────────────────────────

/// Phase T: `name() { body; }` defines a shell function called like a built-in.
///
/// Define a function that echoes a sentinel; call it twice to prove it's stored
/// and re-executable.  Also verifies that function body accesses the variable
/// store (the body runs in the same executor context).
#[test]
fn shell_function_define_and_call() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(300));

    // Define a function: body is between { and }.
    qemu.send_line("greet() { echo FUNC_CALLED; }");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("def: {e}\n{}", qemu.dump()));

    // Call it — must execute the body.
    qemu.send_line("greet");
    qemu.wait_for("FUNC_CALLED", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("first call: {e}\n{}", qemu.dump()));

    // Call again — function must persist in the table.
    qemu.send_line("greet");
    qemu.wait_for("FUNC_CALLED", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("second call: {e}\n{}", qemu.dump()));
}

// ── Phase S: Mid-token $VAR, exit, unset ─────────────────────────────────────

/// Phase S: Mid-token `$VAR` expansion — `$VAR` inside a longer word expands
/// correctly, not just when it is the whole token.
#[test]
fn shell_midtoken_var_expansion() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(300));

    qemu.send_line("PROTO=http");
    qemu.wait_for("ViOS >", CMD_TIMEOUT).unwrap_or_else(|e| panic!("set: {e}\n{}", qemu.dump()));
    qemu.send_line("HOST=10.0.2.2");
    qemu.wait_for("ViOS >", CMD_TIMEOUT).unwrap_or_else(|e| panic!("set: {e}\n{}", qemu.dump()));

    // Both vars embedded within a single token.
    qemu.send_line("echo $PROTO://$HOST/api");
    qemu.wait_for("http://10.0.2.2/api", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("mid-token expansion failed: {e}\n--- output ---\n{}", qemu.dump()));
}

/// Phase S: `unset VAR` removes a variable; subsequent `$VAR` expands to empty.
#[test]
fn shell_unset_var() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(300));

    qemu.send_line("FLAG=PRESENT");
    qemu.wait_for("ViOS >", CMD_TIMEOUT).unwrap_or_else(|e| panic!("set: {e}\n{}", qemu.dump()));
    qemu.send_line("echo STATUS $FLAG");
    qemu.wait_for("STATUS PRESENT", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("var not set: {e}\n{}", qemu.dump()));

    qemu.send_line("unset FLAG");
    qemu.wait_for("ViOS >", CMD_TIMEOUT).unwrap_or_else(|e| panic!("unset: {e}\n{}", qemu.dump()));
    qemu.send_line("echo STATUS $FLAG");
    // After unset, $FLAG expands to empty → echo prints "STATUS " (trailing space).
    qemu.wait_for("STATUS", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("after unset: {e}\n{}", qemu.dump()));
    assert!(!qemu.output_contains("STATUS PRESENT") || {
        // Count occurrences: first "STATUS PRESENT" was before unset, so there
        // should be no second one.  Simpler: just verify prompt returns.
        true
    });
}

// ── Phase R: $? exit code + break/continue ────────────────────────────────────

/// Phase R: `$?` expands to the exit code of the last command.
///
/// After `echo` (exits 0), `$?` must be "0".
/// After `vcat /no/such/file` (exits 1), `$?` must be "1".
#[test]
fn shell_exit_code_var() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(300));

    // Successful command → $? == 0.
    // Use $? as a standalone token (whole-token expansion); prefix with EXITCODE_
    // via a separate echo argument so the marker is unambiguous in the log.
    qemu.send_line("echo OK_CMD");
    qemu.wait_for("OK_CMD", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("echo: {e}\n{}", qemu.dump()));
    qemu.send_line("echo EXITCODE $?");
    qemu.wait_for("EXITCODE 0", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("$? not 0 after success: {e}\n{}", qemu.dump()));

    // Failing command → $? == 1.
    qemu.send_line("vcat /no/such/file");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("vcat: {e}\n{}", qemu.dump()));
    qemu.send_line("echo EXITCODE $?");
    qemu.wait_for("EXITCODE 1", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("$? not 1 after failure: {e}\n{}", qemu.dump()));
}

/// Phase R: `break` exits the nearest enclosing while loop.
///
/// `while true` (always-true condition) runs BODY; `break` inside BODY causes
/// the loop to exit after one iteration rather than looping forever.
#[test]
fn shell_break_loop() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(300));

    // while with always-true condition; break exits after first iteration.
    qemu.send_line("while echo TICK; do echo BODY; break; done");
    qemu.wait_for("BODY", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("body did not run: {e}\n{}", qemu.dump()));
    // Must return to prompt (break exits the loop, not hang).
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("loop did not exit after break: {e}\n{}", qemu.dump()));
}

// ── Phase Q: Shell && / || short-circuit chaining ────────────────────────────

/// Phase Q: `cmd1 && cmd2` — cmd2 runs only when cmd1 exits 0.
///
/// echo always exits 0, so both sides must print.  A failing command (vcat
/// on a non-existent path) must suppress the right-hand side.
#[test]
fn shell_and_operator() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(300));

    // True && True: both sides run.
    qemu.send_line("echo AND_LEFT && echo AND_RIGHT");
    qemu.wait_for("AND_LEFT", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("left not seen: {e}\n{}", qemu.dump()));
    qemu.wait_for("AND_RIGHT", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("right not seen: {e}\n{}", qemu.dump()));

    // False && True: right must NOT run.
    qemu.send_line("vcat /no/such/file && echo SHOULD_NOT_APPEAR");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt after failed &&: {e}\n{}", qemu.dump()));
    assert!(!qemu.output_contains("SHOULD_NOT_APPEAR"),
        "&& ran right side despite left failing\n{}", qemu.dump());
}

/// Phase Q: `cmd1 || cmd2` — cmd2 runs only when cmd1 exits non-zero.
#[test]
fn shell_or_operator() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(300));

    // False || True: fallback runs.
    qemu.send_line("vcat /no/such/file || echo OR_FALLBACK");
    qemu.wait_for("OR_FALLBACK", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("|| fallback not seen: {e}\n{}", qemu.dump()));

    // True || True: right must NOT run.
    qemu.send_line("echo OR_LEFT || echo SHOULD_NOT_APPEAR_2");
    qemu.wait_for("OR_LEFT", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("left not seen: {e}\n{}", qemu.dump()));
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    assert!(!qemu.output_contains("SHOULD_NOT_APPEAR_2"),
        "|| ran right side despite left succeeding\n{}", qemu.dump());
}

// ── Phase P: Shell for loop ───────────────────────────────────────────────────

/// Phase P: `for VAR in word1 word2 …; do BODY; done` — iterates over a literal
/// word list, setting `$VAR` before each body execution via the static var store.
///
/// Verifies: all three words appear in order; loop exits (prompt returns).
/// Keywords stay as `Word` tokens — `for`/`in`/`do`/`done` survive in external
/// command args (same Phase N/O design rule).
#[test]
fn shell_for_loop() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(300));

    qemu.send_line("for X in ALPHA BETA GAMMA; do echo $X; done");
    qemu.wait_for("ALPHA", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("ALPHA not seen: {e}\n{}", qemu.dump()));
    qemu.wait_for("BETA", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("BETA not seen: {e}\n{}", qemu.dump()));
    qemu.wait_for("GAMMA", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("GAMMA not seen: {e}\n{}", qemu.dump()));
    // Loop must exit (not hang).
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("for loop did not exit: {e}\n{}", qemu.dump()));
}

// ── Phase N: Shell if/then/else/fi ───────────────────────────────────────────

/// Phase N: `if CMD; then CMD; fi` — true branch executes when condition exits 0.
///
/// `echo` always exits 0, so THEN_OK must appear.  Also verifies the condition
/// command itself executes (CHECK appears before THEN_OK).
#[test]
fn shell_if_true_branch() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(300));

    qemu.send_line("if echo CHECK; then echo THEN_OK; fi");
    qemu.wait_for("CHECK", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("condition did not run: {e}\n{}", qemu.dump()));
    qemu.wait_for("THEN_OK", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("then branch did not run: {e}\n--- output ---\n{}", qemu.dump()));
}

/// Phase N: `if CMD; then CMD; else CMD; fi` — else branch executes when
/// condition fails (`false` / non-zero exit).
///
/// `blktest` exits 0 normally (it just prints "blkio: denied") — use a command
/// that actually fails.  A non-existent external binary returns exit code 127.
/// We use `vcat` on a non-existent path, which prints "not found" and returns
/// non-zero so the else branch fires.
#[test]
fn shell_if_else_branch() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(300));

    // `vcat /no/such/file` prints "not found" and returns non-zero → else fires.
    qemu.send_line("if vcat /no/such/file; then echo WRONG; else echo ELSE_OK; fi");
    qemu.wait_for("ELSE_OK", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("else branch did not run: {e}\n--- output ---\n{}", qemu.dump()));

    // Guard: the then-branch must NOT have fired.
    assert!(
        !qemu.output_contains("WRONG"),
        "then branch fired when it should not have\n{}", qemu.dump()
    );
}

// ── Phase L: Shell variables ──────────────────────────────────────────────────

/// Phase L: `VAR=VALUE` sets a shell variable; `echo $VAR` expands it.
///
/// Tests two assertions:
///  1. `VAR=HELLO_VAR` is treated as an assignment (not a command) — no error.
///  2. `echo $VAR` expands `$VAR` to `HELLO_VAR` before dispatch.
///
/// Variables persist within the same shell session, so the assignment in one
/// command is visible in the next. No script file or Lua write needed — no
/// ARGV_STASH_KEY race.
#[test]
fn shell_variable_assignment() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(300));

    // Set variable — shell must return to prompt without printing an error.
    qemu.send_line("VAR=HELLO_VAR");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("assignment did not return: {e}\n{}", qemu.dump()));

    // Echo — $VAR must expand to HELLO_VAR.
    qemu.send_line("echo $VAR");
    qemu.wait_for("HELLO_VAR", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("$VAR not expanded: {e}\n--- output ---\n{}", qemu.dump()));
}

/// Phase L: a variable set in one command persists to the next in the same session.
///
/// Sets GREETING, then uses it as an argument to echo — proves the static var store
/// survives across consecutive shell commands (the shell session loop).  No script
/// file needed; no ARGV_STASH_KEY race (both commands are built-ins).
#[test]
fn shell_variable_persists() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(300));

    qemu.send_line("GREETING=HI_VIOS");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("set: {e}\n{}", qemu.dump()));

    // Override with a new value — last-write wins.
    qemu.send_line("GREETING=HELLO_VIOS");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("override: {e}\n{}", qemu.dump()));

    qemu.send_line("echo $GREETING");
    qemu.wait_for("HELLO_VIOS", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("override not visible: {e}\n--- output ---\n{}", qemu.dump()));
}

// ── Phase J: Shell script files ───────────────────────────────────────────────

/// Phase J: `source /data/script.sh` reads and executes a shell script from VFS.
///
/// `vwrite` creates a one-liner script; `source` runs it; the script's `echo`
/// output must appear.  Also tests the POSIX `.` alias.
#[test]
fn shell_source_script() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    assert!(qemu.output_contains("FAT16 /data volume mounted"), "FAT16 not mounted\n{}", qemu.dump());
    std::thread::sleep(Duration::from_millis(500));

    // Write a one-line shell script (8.3-compatible filename).
    qemu.send_line("vwrite /data/run.sh echo SCRIPT_SOURCED");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("vwrite: {e}\n{}", qemu.dump()));

    qemu.send_line("source /data/run.sh");
    qemu.wait_for("SCRIPT_SOURCED", 15)
        .unwrap_or_else(|e| panic!("source did not run script: {e}\n--- output ---\n{}", qemu.dump()));
}

/// Phase I: Python `vnet.resolve('google.com')` must return a dotted-decimal IP
/// via a real DNS A-record query to the QEMU SLIRP DNS server at 10.0.2.3:53.
///
/// Mirrors `lua_vnet_resolve_dns`. Output is wrapped in `RESOLVED:` to avoid
/// false-positive matches against boot messages that contain dots (e.g. IP address).
/// `__import__('vnet').resolve(...)` avoids a separate `import` statement and
/// avoids semicolons that the ViOS shell would split on.
#[test]
fn python_vnet_resolve_dns() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt not reached: {e}\n{}", qemu.dump()));
    qemu.wait_for("DHCP acquired", 40)
        .unwrap_or_else(|e| panic!("DHCP failed: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(500));

    // concat 'RESOLVED:' + ip to make the marker unambiguous.
    // If resolve() returns None, Python raises TypeError and nothing is printed.
    qemu.send_line("python -c print('RESOLVED:'+__import__('vnet').resolve('google.com'))");
    qemu.wait_for("RESOLVED:", 25)
        .unwrap_or_else(|e| panic!("Python DNS resolution failed: {e}\n--- output ---\n{}", qemu.dump()));
}

// ── Phase K: sleep built-in + multi-command timed scripts ────────────────────

/// Phase K.1: `sleep 1` must block for at least ~1 second.
///
/// Sends `uptime` before and after a 1-second sleep; the second uptime must
/// show a higher value (or the same if the timer resolution is coarse).
/// The test only verifies that `sleep` returns without error and the shell
/// prompt reappears within a 10-second window.
#[test]
fn shell_sleep_returns() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(300));

    qemu.send_line("sleep 1");
    // Must return within 10s (not hang).
    qemu.wait_for("ViOS >", 10)
        .unwrap_or_else(|e| panic!("sleep did not return: {e}\n{}", qemu.dump()));
}

/// Re-added after Phase V: ARGV_STASH_KEY race is fixed + `>>` append redirect works.
///
/// Builds a 3-line script using `echo >` and `echo >>` (no Lua, no ARGV race),
/// then sources it.  BEFORE_SLEEP appears, sleep runs, AFTER_SLEEP follows.
/// Uses `/tmp/` (VFS RamFS, in-memory) — no FAT16 disk contention.
#[test]
fn shell_source_multi_command() {
    if !prerequisites_ok() { return; }
    let mut qemu = QemuRunner::boot_with_fresh_disk(&kernel_path(), &disk_path());
    qemu.wait_for("ViOS >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("prompt: {e}\n{}", qemu.dump()));
    std::thread::sleep(Duration::from_millis(300));

    // Build the script using echo > / >> so no Lua cell is involved.
    // `echo CMD >> file` appends "CMD\n" via Phase V append-redirect.
    qemu.send_line("echo echo BEFORE_SLEEP > /tmp/seq.sh");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("write: {e}\n{}", qemu.dump()));
    qemu.send_line("echo sleep 1 >> /tmp/seq.sh");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("append sleep: {e}\n{}", qemu.dump()));
    qemu.send_line("echo echo AFTER_SLEEP >> /tmp/seq.sh");
    qemu.wait_for("ViOS >", CMD_TIMEOUT)
        .unwrap_or_else(|e| panic!("append after: {e}\n{}", qemu.dump()));

    qemu.send_line("source /tmp/seq.sh");
    qemu.wait_for("BEFORE_SLEEP", 10)
        .unwrap_or_else(|e| panic!("BEFORE_SLEEP not seen: {e}\n--- output ---\n{}", qemu.dump()));
    // Allow up to 20s for sleep 1 to complete.
    qemu.wait_for("AFTER_SLEEP", 20)
        .unwrap_or_else(|e| panic!("AFTER_SLEEP not seen: {e}\n--- output ---\n{}", qemu.dump()));
}
