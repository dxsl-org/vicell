# Phase 1: Add `SOCKET_STATE (0x19)` opcode to net Cell

## Context Links

- Plan: [plan.md](plan.md)
- Net cell main loop + dispatch: `cells/services/net/src/main.rs:63-320`
- Opcode constants: `cells/services/net/src/poll_driver.rs:21-42`
- Socket table (handle + state lookup): `cells/services/net/src/socket_table.rs`
- smoltcp `tcp::State`: imported via `socket::tcp` (`main.rs:28`)

## Overview

- **Priority:** P1 (blocks Phase 2)
- **Status:** pending
- **Description:** Add a read-only opcode that returns the live smoltcp TCP
  state of a socket as a single byte, so consumers can disambiguate a 0-byte
  RECV (no data yet vs. FIN received / connection closed).

## Key Insights

- `0x19` is the only free value between the existing user opcodes (`0x10–0x18`)
  and `GET_LOCAL_IP (0x20)`. Verified `poll_driver.rs:21-42`.
- The handler already has `sockets` and `table` in scope — `socket.state()`
  returns `smoltcp::socket::tcp::State` directly (smoltcp public API). No new
  imports beyond what `main.rs:28` (`socket::tcp`) already brings in.
- `iface.poll(...)` runs **before** every CellRequest handler (`main.rs:175`),
  so by the time SOCKET_STATE reads `socket.state()`, smoltcp has already
  processed any inbound FIN — the state is fresh.
- This is a pure read; it MUST NOT mutate `table` state. (Unlike SEND/RECV which
  promote `Connecting → Connected`.) Keep it side-effect free to honor KISS.

## Requirements

**Functional**
- New constant `SOCKET_STATE: u8 = 0x19` in `cell_opcodes`.
- New match arm in `handle_socket_syscall` that:
  - Looks up the handle via `table.get(cap)`.
  - Reads `sockets.get_mut::<tcp::Socket>(handle).state()`.
  - Maps the state to one byte (table below) and replies `sys_send(sender, &[byte])`.
  - On unknown cap: reply `[0x00]` (Closed) — a missing socket is effectively closed.

**Non-functional**
- No new heap allocation in the handler (1-byte stack reply).
- `#![forbid(unsafe_code)]` posture preserved (no unsafe added).
- **Exhaustive match, no fallback.** `smoltcp 0.11.0` `tcp::State` is NOT
  `#[non_exhaustive]` and has exactly 11 variants (verified `tcp.rs:108-122`).
  A `_ => 0xFF` arm is an unreachable pattern — `cargo clippy -D warnings`
  treats `unreachable_patterns` as an error. **Drop the catch-all entirely.**
  If a future smoltcp bump adds a variant, the compiler forces an explicit
  addition — that is better than silently returning 0xFF.

## Architecture

### Data flow

```
curl ── [0x19][cap:8] ──► net cell IPC recv (main.rs:138)
                          decode_message → CellRequest{0x19,cap,[]} (poll_driver.rs:70)
                          iface.poll()  (main.rs:175)  ← refreshes TCP state
                          handle_socket_syscall(0x19,...)
                            table.get(cap) → SocketHandle
                            sockets.get_mut::<tcp::Socket>(h).state()
                            map State → byte
                          sys_send(sender, &[byte])  ──► curl
                          iface.poll()  (main.rs:177)
```

### State → byte mapping (frozen contract)

| smoltcp `tcp::State` | byte | Meaning for curl |
|----------------------|------|------------------|
| Closed        | 0x00 | done / dead — stop reading |
| Listen        | 0x0A | (server only; not used by curl) |
| SynSent       | 0x01 | handshaking |
| SynReceived   | 0x02 | handshaking |
| Established   | 0x03 | open — keep reading on 0-byte RECV |
| FinWait1      | 0x04 | we closed; draining |
| FinWait2      | 0x05 | we closed; draining |
| CloseWait     | 0x06 | **server sent FIN — body complete** |
| Closing       | 0x07 | both closing |
| LastAck       | 0x08 | final ack |
| TimeWait      | 0x09 | done |
| (unknown)     | 0xFF | treat as keep-waiting w/ loop cap |

## Related Code Files

**Modify**
- `cells/services/net/src/poll_driver.rs` — add `SOCKET_STATE` const (~1 line).
- `cells/services/net/src/main.rs` — add match arm in `handle_socket_syscall`
  (insert before the catch-all `_ => { sys_send(sender, &[]); }` at `main.rs:316`).

**Create / Delete:** none.

## Implementation Steps

1. In `poll_driver.rs`, inside `pub mod cell_opcodes`, after `ACCEPT` (line 39)
   add:
   ```rust
   /// Query live TCP state of a socket; reply = 1-byte state code.
   pub const SOCKET_STATE: u8 = 0x19;
   ```

2. In `main.rs`, add a small free helper above `handle_socket_syscall` (or as a
   local closure) that maps state to byte:
   ```rust
   /// Map a smoltcp TCP state to the 1-byte wire encoding consumers expect.
   ///
   /// No `_ =>` arm: `tcp::State` is exhaustive in smoltcp 0.11 (11 variants).
   /// A wildcard arm would be unreachable and fail `clippy -D warnings`.
   fn tcp_state_byte(s: tcp::State) -> u8 {
       match s {
           tcp::State::Closed      => 0x00,
           tcp::State::SynSent     => 0x01,
           tcp::State::SynReceived => 0x02,
           tcp::State::Established => 0x03,
           tcp::State::FinWait1    => 0x04,
           tcp::State::FinWait2    => 0x05,
           tcp::State::CloseWait   => 0x06,
           tcp::State::Closing     => 0x07,
           tcp::State::LastAck     => 0x08,
           tcp::State::TimeWait    => 0x09,
           tcp::State::Listen      => 0x0A,
       }
   }
   ```

3. In `handle_socket_syscall`, add the arm before the catch-all (`main.rs:316`):
   ```rust
   cell_opcodes::SOCKET_STATE => {
       // Read-only: must NOT mutate table state.
       let byte = match table.get(cap) {
           Some(handle) => {
               let socket = sockets.get_mut::<tcp::Socket>(handle);
               tcp_state_byte(socket.state())
           }
           None => 0x00, // unknown cap == effectively closed
       };
       sys_send(sender, &[byte]);
   }
   ```
   Note `payload` is unused for this opcode — that is fine (the existing
   BIND/LISTEN arm already does `let _ = (cap, payload);`).

4. Verify it compiles:
   ```
   cargo check -p service-net --target riscv64gc-unknown-none-elf
   ```

## Todo List

- [ ] Add `SOCKET_STATE = 0x19` to `cell_opcodes` (`poll_driver.rs`).
- [ ] Add `tcp_state_byte` helper (`main.rs`).
- [ ] Add `SOCKET_STATE` match arm before catch-all (`main.rs:316`).
- [ ] `cargo check -p service-net` clean.

## Success Criteria

- `cargo check -p service-net --target riscv64gc-unknown-none-elf` exits 0.
- New arm placed before `_ => { sys_send(sender, &[]); }` (else it is dead code).
- No mutation of `table` in the new arm (grep the arm body for `set_state`/`remove` → none).
- Reply is exactly 1 byte for any cap (existing or missing).

## Evidence

**Completion Status**: ✅ Complete (2026-06-03)

**Build Verification**:
```
cargo check -p service-net --target riscv64gc-unknown-none-elf
```
**Result**: Exit 0 (clean). 1 pre-existing warning (unrelated to Phase 1 changes).

**Code Changes**:
- Added `SOCKET_STATE = 0x19` to `cells/services/net/src/poll_driver.rs` (line ~40)
- Added `tcp_state_byte()` helper function in `cells/services/net/src/main.rs` (exhaustive match, no wildcard)
- Added `SOCKET_STATE` match arm in `handle_socket_syscall()` before catch-all (line ~312)
- All 11 TCP states mapped correctly; missing caps return `0x00`

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| smoltcp adds a `tcp::State` variant in a future upgrade | Low | Low | Compiler error on upgrade forces explicit handling; no silent `_ =>` fallback |
| Arm placed after catch-all → never reached | Low | High | Insert explicitly before `main.rs:316`; verify with grep that 0x19 arm precedes `_ =>` |
| `state()` requires `get_mut` borrow conflicting with poll | Low | Low | Handler runs between the two `iface.poll` calls; borrows are scoped to the arm |
| 1-byte reply mis-sized (e.g. `&byte` vs `&[byte]`) | Low | Medium | Use `&[byte]` slice; matches CLOSE's `&[0u8]` pattern |

## Security Considerations

- Read-only opcode; no new authority granted. A cell can already CONNECT/SEND/RECV
  on its own caps — exposing the state of a cap it owns adds no privilege.
- Unknown-cap returns `0x00` rather than leaking whether a cap exists for another
  cell; the cap namespace is already per-`SocketTable` (process-shared net cell),
  so no cross-cell info leak beyond what RECV already exposes.

## Next Steps

- Unblocks Phase 2 (`curl` done-detection loop).
- The state-byte contract in this file is the frozen interface Phase 2 codes to.
