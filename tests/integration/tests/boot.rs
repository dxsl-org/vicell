//! End-to-end boot + interactive tests driven through QEMU serial.
//!
//! These require `qemu-system-riscv64` on PATH and pre-built artifacts:
//!   cargo build --release -p vios-kernel
//!   ./gen_disk.ps1
//!
//! Paths are relative to the repo root (two levels up from this crate). The
//! tests resolve them from CARGO_MANIFEST_DIR so they run regardless of cwd.

use std::path::PathBuf;
use vios_integration_tests::{qemu_binary, spawn_echo_server, spawn_http_server, QemuRunner};

const BOOT_TIMEOUT: u64 = 40;

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
    let qemu = QemuRunner::boot(&kernel_path(), &disk_path());
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
    let qemu = QemuRunner::boot(&kernel_path(), &disk_path());
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
    let mut qemu = QemuRunner::boot(&kernel_path(), &disk_path());
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
    let mut qemu = QemuRunner::boot(&kernel_path(), &disk_path());
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
    let mut qemu = QemuRunner::boot(&kernel_path(), &disk_path());
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
    let qemu = QemuRunner::boot(&kernel_path(), &disk_path());
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
    let qemu = QemuRunner::boot(&kernel_path(), &disk_path());
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
    let qemu = QemuRunner::boot(&kernel_path(), &disk_path());
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
    let mut qemu = QemuRunner::boot(&kernel_path(), &disk_path());
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

    let mut qemu = QemuRunner::boot(&kernel_path(), &disk_path());

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

    let mut qemu = QemuRunner::boot(&kernel_path(), &disk_path());

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
