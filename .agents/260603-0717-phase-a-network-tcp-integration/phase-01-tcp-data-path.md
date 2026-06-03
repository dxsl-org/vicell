---
phase: 1
title: "Wire TCP Data-Path"
status: pending
priority: P1
effort: "1-2 sessions"
dependencies: []
---

# Phase 1: Wire TCP Data-Path

## Context Links

- Scout report: `.agents/reports/` (network service detailed scout)
- Stub location: [`cells/services/net/src/main.rs:197-202`](../../cells/services/net/src/main.rs)
- Socket table: [`cells/services/net/src/socket_table.rs`](../../cells/services/net/src/socket_table.rs)
- smoltcp docs: https://docs.rs/smoltcp/0.11

## Overview

The net Cell's `handle_socket_syscall()` stubs CONNECT/SEND/RECV/BIND/LISTEN/ACCEPT
to return `0xFF`. smoltcp's TCP socket (`smoltcp::socket::tcp::Socket`) is already
allocated per SOCKET_TCP with 4 KB RX and TX buffers — just not wired to IPC opcodes.

This phase implements CONNECT and SEND/RECV for outbound TCP, plus a SocketState
enum to guard against misuse (double-connect, send before connect, etc.).

## File Inventory

| File | Action | Lines Δ | Test Impact |
|------|--------|---------|-------------|
| `cells/services/net/src/main.rs` | Modify | +80 | TCP ops now functional |
| `cells/services/net/src/socket_table.rs` | Modify | +40 | State tracking per socket |
| `cells/services/net/src/socket_state.rs` | Create | +60 | New: SocketState enum |

## Key Insights (from scout)

- `handle_socket_syscall()` at `main.rs:162-208` dispatches by opcode. Stubs are at lines 197-202.
- `SocketTable::get()` at `socket_table.rs:44` is already written but `#[allow(dead_code)]` — waiting for SEND/RECV.
- smoltcp's `tcp::Socket` has `connect(cx, remote, local_port)`, `send_slice(&[u8])`, `recv_slice(&mut [u8])` — clean API, no async needed for polling model.
- The net Cell runs a polling loop (100 ms interval). TCP state machine advances via `iface.poll()` call at `main.rs:78-80`. No special threading needed.
- `Instant` for smoltcp comes from `smoltcp::time::Instant::from_millis(sys_get_time() / 10_000)` (mtime ticks → ms).

## Architecture

```
Consumer Cell                 Net Cell                      smoltcp
─────────────────────         ──────────────────────────    ──────────────
sys_send(net_cell, CONNECT)
  payload: addr[4] port[2] ─→ handle_socket_syscall()
                                lookup SocketHandle in table
                                socket.connect(cx, remote, local)
                                reply 0x00 ok / 0x01 err ─────────────────→
                                                           iface.poll() on
                                                           next loop tick
                                                           → TCP SYN sent
sys_send(net_cell, SEND)
  cap[8] + data ─────────────→ socket.send_slice(data)
                                reply bytes_written ───────→ smoltcp buffers
sys_send(net_cell, RECV)
  cap[8] + buf_len[4] ────────→ socket.recv_slice(&mut buf[..len])
                                reply data bytes ──────────→ smoltcp drains RX
```

## IPC Message Format

From `poll_driver.rs`, messages are: `[opcode:1][cap:8][payload:*]`

| Opcode | Request Payload | Reply |
|--------|----------------|-------|
| CONNECT (0x12) | `addr[4] port[2]` | `[0x00]` ok / `[0x01]` err |
| SEND (0x13) | data bytes | `[n:4 LE]` bytes written |
| RECV (0x14) | `buf_len[4 LE]` | data bytes (0 bytes = no data yet) |
| LISTEN (0x17) | `port[2]` | `[cap:8 LE]` listener CapId |
| ACCEPT (0x18) | — | `[cap:8 LE]` stream CapId / `[0xFF]` no conn |
| CLOSE (0x15) | — | `[0x00]` (already implemented) |

## Socket State Enum

```rust
// socket_state.rs — new file
pub enum SocketState {
    Created,     // SOCKET_TCP created, not yet connected
    Connecting,  // CONNECT sent, TCP SYN in flight
    Connected,   // TCP handshake complete
    Listening,   // LISTEN called, waiting for SYN
    Closed,      // CLOSE called or RST received
}
```

Add `state: SocketState` field to `SocketTable` entries (or a parallel `BTreeMap<u64, SocketState>`).

## Implementation Steps

### Step 0: Verify smoltcp 0.11 TCP socket API (MANDATORY before coding)

Run locally before writing any CONNECT/SEND/RECV code:

```bash
# Check exact signature in the vendored source
grep -n "pub fn connect\|pub fn send_slice\|pub fn recv_slice" \
  $(cargo metadata --format-version 1 | python -c "import sys,json; \
  [print(p['manifest_path'].replace('Cargo.toml','src/socket/tcp.rs')) \
  for p in json.load(sys.stdin)['packages'] if p['name']=='smoltcp']") 2>/dev/null
```

Or check `target/` for the cached source. Specifically confirm:
- `tcp::Socket::connect(cx: &mut Context, remote: impl Into<IpEndpoint>, local: impl Into<IpEndpoint>)` — returns `Result<(), ConnectError>`
- `tcp::Socket::send_slice(&mut self, data: &[u8])` — returns `Result<usize, SendError>`
- `tcp::Socket::recv_slice(&mut self, data: &mut [u8])` — returns `Result<usize, RecvError>`
- `tcp::Socket::can_send(&self) -> bool`
- `tcp::Socket::can_recv(&self) -> bool`
- `tcp::Socket::state(&self) -> tcp::State`

If signatures differ from above, adjust the implementation steps accordingly.

### Step 1: Create `socket_state.rs`

```rust
// cells/services/net/src/socket_state.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketState { Created, Connecting, Connected, Listening, Closed }
```

### Step 2: Extend `SocketTable` to track state

In `socket_table.rs`, add `states: BTreeMap<u64, SocketState>` alongside `entries`.
Add `set_state(cap, state)` and `get_state(cap) -> Option<SocketState>`.

### Step 3: Implement CONNECT in `handle_socket_syscall()`

Note: `handle_socket_syscall` receives `cap: u64` and `payload: &[u8]` already
split by `decode_message()`. Do NOT re-index `msg` — use the passed parameters.

```rust
cell_opcodes::CONNECT => {
    if payload.len() < 6 { sys_send(sender, &[0x01]); return; }
    // cap already decoded from message header by decode_message()
    let addr = [payload[0], payload[1], payload[2], payload[3]];
    let port = u16::from_le_bytes([payload[4], payload[5]]);

    if let Some(handle) = socket_table.get(cap) {
        let socket = sockets.get_mut::<tcp::Socket>(handle);
        let remote = IpEndpoint::new(IpAddress::v4(addr[0], addr[1], addr[2], addr[3]), port);
        // smoltcp needs an unused local port (ephemeral range)
        let local_port = next_ephemeral_port(); // 49152..65535
        let cx = iface.context();
        match socket.connect(cx, remote, local_port) {
            Ok(()) => {
                socket_table.set_state(cap, SocketState::Connecting);
                sys_send(sender, &[0x00]);
            }
            Err(_) => sys_send(sender, &[0x01]),
        }
    } else {
        sys_send(sender, &[0x01]);
    }
}
```

### Step 4: Implement SEND

```rust
cell_opcodes::SEND => {
    // cap and payload already split by decode_message()
    let data = payload; // payload = bytes after [opcode][cap:8]

    if let Some(handle) = socket_table.get(cap) {
        // Advance TCP state machine first
        let cx = iface.context();
        let socket = sockets.get_mut::<tcp::Socket>(handle);
        if socket.can_send() {
            let n = socket.send_slice(data).unwrap_or(0);
            // reply: bytes written as 4-byte LE
            sys_send(sender, &(n as u32).to_le_bytes());
        } else {
            sys_send(sender, &0u32.to_le_bytes()); // 0 = not ready yet
        }
    } else {
        sys_send(sender, &0u32.to_le_bytes());
    }
}
```

### Step 5: Implement RECV

```rust
cell_opcodes::RECV => {
    // cap already decoded by decode_message()
    let buf_len = if payload.len() >= 4 {
        u32::from_le_bytes(payload[0..4].try_into().unwrap()) as usize
    } else { 512 };
    let buf_len = buf_len.min(4096); // cap at 4 KB

    if let Some(handle) = socket_table.get(cap) {
        let socket = sockets.get_mut::<tcp::Socket>(handle);
        let mut data = alloc::vec![0u8; buf_len];
        let n = if socket.can_recv() {
            socket.recv_slice(&mut data).unwrap_or(0)
        } else { 0 };
        sys_send(sender, &data[..n]); // 0-byte reply = no data yet
    } else {
        sys_send(sender, &[]);
    }
}
```

### Step 6: Ephemeral port allocator

Add to `main.rs`:
```rust
static NEXT_PORT: core::sync::atomic::AtomicU16 =
    core::sync::atomic::AtomicU16::new(49152);

fn next_ephemeral_port() -> u16 {
    let p = NEXT_PORT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    if p >= 65535 { NEXT_PORT.store(49152, core::sync::atomic::Ordering::Relaxed); }
    p
}
```

### Step 7: Fix Instant time units

Live code uses `from_micros(ticks/10)` (not `from_millis(ticks/10_000)`).
Verify the smoltcp Instant formula in the existing main.rs before touching it.
Do NOT change the formula — use whatever is already there for DHCP.

### Step 8: Increase iface.poll() frequency for TCP

The 100 ms poll interval means TCP SYN → SYN/ACK can take up to 200 ms
(SYN sent on one poll, ACK processed on next). For the integration test this
is acceptable. CONNECT reply to caller happens BEFORE the handshake completes —
the socket transitions from `SynSent` to `Established` on a future poll.

**Note for nc caller**: after CONNECT returns `0x00`, the TCP handshake is still
in progress. nc must poll with RECV (which returns 0 bytes) until the socket is
`Established` before SEND will successfully buffer data. The SocketState guard
(`Connecting` → `Connected`) should check `socket.state()` on each RECV request.

### Step 8: `cargo check` the net Cell

```bash
cargo check -p vios-net --target riscv64gc-unknown-none-elf
```

Fix any type errors (smoltcp context API, socket type parameters).

## Test Scenario Matrix

| Scenario | Priority | Covered in Phase |
|----------|----------|-----------------|
| CONNECT to QEMU gateway 10.0.2.2:80 | Critical | Phase 2 |
| SEND 8-byte HTTP GET | Critical | Phase 2 |
| RECV response | Critical | Phase 2 |
| CLOSE socket | Critical | Phase 2 (reuse existing) |
| CONNECT refused (10.0.2.2:9) | High | Phase 2 |
| Double CONNECT same cap | Medium | Phase 1 (state guard) |
| SEND before CONNECT | Medium | Phase 1 (state guard) |
| RECV returns 0 if no data | Medium | Phase 2 |

## Dependency Map

- No dependency on other phases
- Phase 2 depends on Phase 1 completing

## Todo List

- [ ] Create `cells/services/net/src/socket_state.rs`
- [ ] Extend `SocketTable` with state tracking
- [ ] Implement CONNECT opcode
- [ ] Implement SEND opcode
- [ ] Implement RECV opcode
- [ ] Add ephemeral port allocator
- [ ] `cargo check` net Cell — zero errors
- [ ] Manual smoke test: shell → `netcat 10.0.2.2 80` or similar Lua script

## Success Criteria

- [ ] `cargo check -p vios-net` passes
- [ ] CONNECT to 10.0.2.2 returns `0x00` (no RST from SLIRP for port 80)
- [ ] SEND returns byte count > 0
- [ ] RECV returns data or 0-byte if not ready
- [ ] Double-CONNECT returns error (state guard works)
- [ ] Phase 2 integration test passes

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| smoltcp `connect()` signature mismatch with 0.11 API | Medium | Low | Check docs.rs for exact signature before coding |
| TCP handshake doesn't complete in 100ms poll interval | Low | Medium | Reduce poll interval to 20ms for test runs; SLIRP is fast |
| SEND before TCP `Established` → 0 bytes written | Low | Low | State guard + retry guidance in docs |
| Port exhaustion (ephemeral wraparound) | Very Low | Low | QEMU SLIRP closes sockets quickly; 16K ports is sufficient |

## Security Considerations

- CONNECT payload must validate addr + port are present (len ≥ 6 check)
- buf_len from RECV capped at 4 KB (prevents OOM from malicious caller)
- Ephemeral port counter uses `Relaxed` ordering — single-core kernel, no race
