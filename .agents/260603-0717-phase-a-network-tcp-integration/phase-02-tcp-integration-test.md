---
phase: 2
title: "TCP Integration Test"
status: pending
priority: P1
effort: "1 session"
dependencies: [1]
---

# Phase 2: TCP Integration Test

## Context Links

- Test harness: [`tests/integration/src/lib.rs`](../../tests/integration/src/lib.rs)
- Existing tests: [`tests/integration/tests/boot.rs`](../../tests/integration/tests/boot.rs)
- Shell IPC pattern: [`cells/apps/shell/src/cmd_fs.rs:10`](../../cells/apps/shell/src/cmd_fs.rs) — `VFS_ENDPOINT = 2`

## Red Team Fixes Applied

Three FATAL issues from review, resolved as follows:

| Issue | Fix |
|-------|-----|
| ServiceLookup stub (always returns 0) | Implement it in kernel OR use hardcoded net Cell ID verified from boot log |
| SLIRP port 23 has no echo server | Use Lua script targeting QEMU's built-in TFTP (UDP) or CONNECT-only test |
| nc.rs API errors (ostd::args, sys_send return type) | Use Lua script instead — Lua FFI is already proven working |

## Overview

Instead of writing a new `nc` binary with raw IPC (complex, fragile, would require
fixing ServiceLookup), this phase uses a **Lua script** injected from the integration
test harness. Lua is already verified working (Phase 18 passes). A Lua script can call
`os.execute` which routes through `sys_spawn_from_path` — but more directly, we can add
a Lua binding to the net Cell IPC and test via `lua -e "..."` from the shell.

**Simpler alternative** (used here): test CONNECT + CLOSE without data transfer.
This verifies the TCP handshake works (SYN/SYN-ACK/ACK via SLIRP) without needing
an echo server. A successful CONNECT to `10.0.2.2:80` (QEMU gateway) returns `0x00`
since SLIRP forwards TCP to the host. A refused port returns `0x01` after RST.

## File Inventory

| File | Action | Lines Δ | Notes |
|------|--------|---------|-------|
| `kernel/src/task/syscall.rs` | Modify | +5 | Implement ServiceLookup (resolve net Cell ID) |
| `kernel/src/task/drivers/registry.rs` | Modify or Create | +20 | Name→task_id registry |
| `tests/integration/tests/boot.rs` | Modify | +20 | Add `network_tcp_socket_create()` test |

## Net Cell Task ID — Verified Approach

From `cells/apps/shell/src/cmd_fs.rs:10`: `VFS_ENDPOINT = 2` (hardcoded, working).
VFS is the first cell spawned by init. Net is the fourth spawn:

```
init = 1  (kernel-assigned)
vfs  = 2  (init's first sys_spawn_from_path)
config = 3
input  = 4
net    = 5
```

**Step 0**: Boot QEMU once, grep serial for `"[net]"` or `"Network Service spawned"`
to confirm the net Cell task ID. Add `log::info!("[net] running as task {}", my_task_id())`
to `cells/services/net/src/main.rs:main()`. Hardcode `NET_ENDPOINT: usize = 5` in nc/test code.

## Implementation Steps

### Step 1: Implement ServiceLookup in kernel

Fix the stub in `kernel/src/task/syscall.rs:640-650`:

```rust
Syscall::ServiceLookup { name_ptr, name_len } => {
    validate_user_buf(name_ptr, name_len, 64)?;
    let name = unsafe {
        core::str::from_utf8(
            core::slice::from_raw_parts(name_ptr as *const u8, name_len)
        ).map_err(|_| SyscallError::InvalidInput)?
    };
    // Match well-known service names to their task IDs
    let id: usize = match name {
        "vfs"    => 2,
        "config" => 3,
        "input"  => 4,
        "net"    => 5,
        "shell"  => 7,
        _        => return Err(SyscallError::FileNotFound),
    };
    Ok(id)
}
```

This is the minimal correct fix — a lookup table matching the init spawn order.
Verify the IDs match boot log output before shipping.

### Step 2: Add net Cell self-identification log

In `cells/services/net/src/main.rs`, before the main loop:
```rust
// Log our task ID so integration tests can verify NET_ENDPOINT constant
log::info!("[net] cell running (task_id logged at spawn)");
```

The kernel already logs `"Init: Network Service spawned."` — check what task ID
`sys_spawn_from_path` returned to init (add `println!("net task_id={}", id)` in init).

### Step 3: Add host-side TCP echo server to test harness

**Why**: QEMU SLIRP user-mode networking forwards guest connections to
`10.0.2.2:PORT` → host's `localhost:PORT`. Run an echo server on the host
during the test. No `-hostfwd` needed — this is the outbound direction.

Add to `tests/integration/src/lib.rs`:

```rust
use std::net::{TcpListener, TcpStream};
use std::io::{Read, Write};
use std::thread;

/// Spawn a simple TCP echo server on an ephemeral port.
/// Returns the port number. Server runs until the returned handle is dropped.
pub fn spawn_echo_server() -> (u16, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("echo server bind");
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
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
    (port, handle)
}
```

### Step 4: Full TCP SEND/RECV integration test

```rust
#[test]
fn network_tcp_send_recv() {
    if !prerequisites_ok() { return; }

    // Start echo server on host BEFORE QEMU boots
    let (echo_port, _echo_handle) = spawn_echo_server();

    let mut qemu = QemuRunner::boot(&kernel_path(), &disk_path());

    qemu.wait_for("ViCell >", BOOT_TIMEOUT)
        .unwrap_or_else(|e| panic!("shell not reached: {e}"));

    qemu.wait_for("DHCP acquired", 20)
        .unwrap_or_else(|e| panic!("DHCP failed: {e}\n{}", qemu.dump()));

    // nc connects to 10.0.2.2:<port> → SLIRP → host echo server
    // nc prints "HELLO_ViCell" echoed back
    qemu.send_line(&format!("nc 10.0.2.2 {echo_port}"));

    qemu.wait_for("connected", 10)
        .unwrap_or_else(|e| panic!("TCP connect failed: {e}\n{}", qemu.dump()));

    qemu.wait_for("HELLO_ViCell", 15)
        .unwrap_or_else(|e| panic!("TCP echo not received: {e}\n{}", qemu.dump()));
}
```

### Step 5: Wire `nc.rs` (same pattern as `cmd_fs.rs:VFS_ENDPOINT`)

```rust
// cells/apps/net-tools/src/bin/nc.rs
// Pattern: identical to cells/apps/shell/src/cmd_fs.rs
// cmd_fs.rs does: sys_send(VFS_ENDPOINT, &buf) then sys_recv(0, &mut reply)
// nc.rs does the same with NET_ENDPOINT

const NET_ENDPOINT: usize = 5; // VERIFY from boot log first (Step 0 in Phase 2)

// Parse args: nc <host_ip> <port>
// Build SOCKET_TCP request → sys_send → sys_recv for cap_id
// Build CONNECT request [0x12][cap:8][addr:4][port:2] → sys_send → sys_recv for ack
// Build SEND request [0x13][cap:8][data] → sys_send → sys_recv for byte count
// Loop RECV [0x14][cap:8][buf_len:4] → sys_send → sys_recv until data arrives
// Print received data to stdout (appears on serial → test harness sees it)
// CLOSE [0x15][cap:8] → sys_send (fire and forget)
```

**Critical**: follow `cmd_fs.rs` pattern exactly. Shell→VFS IPC works in production
tests. nc→net must use the same send/recv sequence.

### Step 6: Rebuild and run

```bash
cargo build --release
./gen_disk.ps1  # regenerate disk with new nc binary

cargo test --manifest-path tests/integration/Cargo.toml \
           --target x86_64-pc-windows-msvc \
           -- network_tcp_send_recv --nocapture
```

### Step 5: Run full test suite

```bash
cargo test --manifest-path tests/integration/Cargo.toml \
           --target x86_64-pc-windows-msvc \
           -- --nocapture 2>&1 | tail -30
```

All 9+ tests should pass.

## Test Scenario Matrix

| Scenario | Priority | Method |
|----------|----------|--------|
| SOCKET_TCP create → non-zero cap | Critical | nc binary (Step 5) |
| CONNECT to host echo server | Critical | network_tcp_send_recv |
| SEND "HELLO_ViCell" + receive echo | Critical | network_tcp_send_recv |
| CLOSE socket cleanly | High | nc binary exit |
| DHCP still passes | Critical | Existing regression test |
| Boot + shell still pass | Critical | Existing regression test |
| ServiceLookup("net") returns correct ID | Medium | Verify via boot log |

## Dependency Map

- Depends on Phase 1 (TCP stubs wired)
- ServiceLookup fix is independent — can be done before Phase 1

## Todo List

- [ ] Boot QEMU, grep serial for net Cell task ID (confirm NET_ENDPOINT constant)
- [ ] Implement ServiceLookup lookup table in `kernel/src/task/syscall.rs`
- [ ] Add `spawn_echo_server()` to `tests/integration/src/lib.rs`
- [ ] Wire `nc.rs` to SOCKET_TCP + CONNECT + SEND + RECV + CLOSE (follow `cmd_fs.rs` pattern)
- [ ] `cargo check -p ViCell-net-tools` — zero errors
- [ ] Add `network_tcp_send_recv` test to `boot.rs`
- [ ] `cargo build --release && ./gen_disk.ps1` — rebuild with new nc binary
- [ ] Run `network_tcp_send_recv` test — passes (HELLO_ViCell echoed)
- [ ] Run full suite — all existing tests still pass

## Success Criteria

- [ ] `network_tcp_send_recv` passes: "HELLO_ViCell" appears in serial output
- [ ] All 9+ existing integration tests still pass (no regressions)
- [ ] nc connects, sends, receives, closes cleanly
- [ ] CONNECT returns `0x00` (not `0xFF` — proves Phase 1 wiring works)

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| Net Cell task ID is not 5 | Medium | Medium | Log ID at boot before hardcoding; fix ServiceLookup |
| sys_send/sys_recv deadlock for nc→net | Low | Fatal | Follow exact `cmd_fs.rs` pattern: same pattern works for shell→vfs |
| SLIRP CONNECT hangs (no RST, no ACK) | Low | Medium | Add timeout: `sys_recv_timeout(0, buf, timeout_ticks=10_000_000)` |
| gen_disk.ps1 doesn't include new nc binary | Low | Low | Verify nc is in `/bin/` in the disk image after rebuild |

## Security Considerations

None specific to integration tests. ServiceLookup returning hardcoded IDs is
acceptable for v0.2 — replace with dynamic registry in v0.3.
