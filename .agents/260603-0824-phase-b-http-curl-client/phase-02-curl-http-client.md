# Phase 2: Implement `curl` HTTP/1.0 GET client

## Context Links

- Plan: [plan.md](plan.md)
- Frozen IPC contract: [plan.md](plan.md) "Frozen IPC contract"
- Opcode added in: [phase-01-socket-state-opcode.md](phase-01-socket-state-opcode.md)
- Working IPC reference (copy patterns from here): `cells/apps/net-tools/src/bin/nc.rs`
- Current stub to replace: `cells/apps/net-tools/src/bin/curl.rs:1-11`
- Shell argv path: `cells/apps/shell/src/executor.rs:175-187`

## Overview

- **Priority:** P1
- **Status:** pending (depends on Phase 1)
- **Description:** Replace the `curl` stub with a real HTTP/1.0 client: parse a
  `http://IP[:PORT][/path]` URL from argv, open a TCP socket via the net cell,
  send a minimal GET, accumulate the full response on the heap until FIN, then
  print the body to serial.

## Key Insights

- **Reuse, don't reinvent.** `nc.rs` already implements: argv parse
  (`sys_spawn_args`, `nc.rs:29-30`), `parse_ipv4` (`nc.rs:153-161`),
  `parse_u16` (`nc.rs:174-183`), SOCKET_TCP→cap (`nc.rs:59-69`), CONNECT
  (`nc.rs:71-86`), retry-SEND-until-buffered (`nc.rs:100-111`), and CLOSE
  (`nc.rs:143-150`). Lift these patterns directly (DRY).
- **Wait-for-Established is implicit in retry-SEND.** SEND replies `n=0` while
  `can_send()` is false (still handshaking — `main.rs:271-276`). Looping SEND
  with the actual HTTP request bytes until `n > 0` both waits for Established
  AND buffers the request. No separate "wait" opcode needed. Exact pattern at
  `nc.rs:100-111`.
- **`sys_recv` returns sender ID, not byte count.** `kernel/src/task.rs:673`
  discards the copy length and returns `Ok(sender_id)`. Use the nc.rs approach:
  zero the receive buffer before each RECV call, then scan for the last non-zero
  byte (`rposition(|&b| b != 0).map(|i| i+1).unwrap_or(0)`) to find the data
  boundary. This is safe for HTTP (headers + ASCII body contain no zero bytes
  in normal usage; for our test body "HELLO" it is guaranteed). Do NOT treat
  `Ok(n)` from `sys_recv` as a byte count.
- **Done-detection needs SOCKET_STATE (0x19).** On a 0-byte RECV, query 0x19.
  `0x06 (CloseWait)` or `0x00 (Closed)` → **drain one final RECV** (server may
  have data buffered alongside FIN), then break. A `can_recv()==false` alone is
  ambiguous; the state check resolves it. Else `sys_yield()` and retry.
- **Argv is a single token.** Shell does `sys_set_spawn_args(&args.join(" "))`
  (`executor.rs:176`); for `curl http://10.0.2.2:8080/` argv = `http://10.0.2.2:8080/`.
  No whitespace splitting needed — parse the whole string as a URL.
- **`extern crate alloc` is valid here.** `ostd` registers the global allocator
  (`libs/ostd/src/heap.rs:52-53`). Use `alloc::vec::Vec<u8>` to accumulate the
  response; do NOT try to size a fixed stack buffer (response > 256 bytes likely).
- **Entry point is `#[no_mangle] pub fn main()`** (matches `nc.rs:26-27`), NOT
  `_start`. The context outline's `#![no_main]` + `_start` mention is wrong for
  this codebase — follow `nc.rs`.

## Requirements

**Functional**
- Parse `http://IP[:PORT][/path]`: strip `http://`, split host[:port] from path
  at first `/`, default port `80`, default path `/`.
- Reject non-`http://` schemes and malformed IPs with a usage message + return.
- Build request: `GET <path> HTTP/1.0\r\nHost: <host>\r\nConnection: close\r\n\r\n`.
- Accumulate full response (headers + body) into `Vec<u8>` until FIN/close.
- Locate `\r\n\r\n` across the FULL accumulation buffer (not per-chunk) and
  print everything after it (the body). Also print the status line so the test
  can match `200`.
- CLOSE the socket on exit (success or error paths).

**Non-functional**
- Loop safety caps: SEND retry ≤ 500 (matches `nc.rs:100`); RECV/state loop
  ≤ 500 iterations to prevent infinite hang if state never reaches close.
- No unsafe (`app-net-tools` is `#![no_std]`, no unsafe today).
- RECV request buffer length ≤ 4096 (net cell caps at 4096 — `main.rs:296`).

## Architecture

### Data flow

```
argv "http://10.0.2.2:8080/path"
  │ parse_url → (host="10.0.2.2", port=8080, path="/path")
  │ parse_ipv4(host) → [10,0,2,2]
  ▼
SOCKET_TCP (0x10)              ──► reply [cap:8]
CONNECT (0x12)[addr:4][port:2] ──► reply [0x00] ok
  │ print "connecting..."
  ▼
loop ≤500: SEND (0x13)[GET req bytes] ──► [n:4]; break when n>0 (Established)
  ▼
response = Vec<u8>::new()
loop ≤500:
  RECV (0x14)[buf_len=2048] ──► chunk (0..n bytes)
  if n>0: response.extend(chunk)
  else:
    SOCKET_STATE (0x19) ──► [state]
    if state==0x06 || state==0x00: break   // CloseWait/Closed → body done
    else: sys_yield()
  ▼
find \r\n\r\n in response → print status line + body (after header end)
CLOSE (0x15)
```

### URL parsing (KISS, hand-rolled — no alloc needed for parse)

```
input: "http://10.0.2.2:8080/index.html"
1. strip prefix "http://" (reject if absent)
2. find first '/' in remainder → splits "10.0.2.2:8080" | "/index.html"
   (no '/' → host_port = whole, path = "/")
3. split host_port on ':' → host="10.0.2.2", port_str="8080" (no ':' → port=80)
4. parse_ipv4(host), parse_u16(port_str)
```

Return borrowed `&str` slices into the argv buffer (which lives for `main`'s
duration in a local `arg_buf` — keep `arg_buf` alive until after SEND builds the
request, OR copy host into a stack array). Simplest: build the request string
eagerly while `arg_buf` is still in scope.

### Request construction

Use `alloc::vec::Vec<u8>` or a fixed stack `[u8; 256]` cursor (HTTP GET headers
fit well under the 512-byte net IPC buffer minus the 9-byte `[opcode][cap]`
prefix → payload budget ~503 bytes; a typical request is < 80 bytes). The full
SEND message is `[0x13][cap:8][request_bytes]`. Keep request ≤ 400 bytes to stay
safely within the 512-byte net IPC buffer.

## Related Code Files

**Modify**
- `cells/apps/net-tools/src/bin/curl.rs` — full rewrite of the stub.

**Create / Delete:** none. (`[[bin]] name = "curl"` already declared,
`Cargo.toml:15-17`.)

## Implementation Steps

1. Header: keep `#![no_std] #![no_main]`, add `extern crate alloc;` and keep
   `extern crate ostd;`. Imports: `use ostd::io::{print, println};` and
   `use ostd::syscall::{sys_recv, sys_send, sys_spawn_args, sys_yield, SyscallResult};`
   plus `use alloc::vec::Vec;`.

2. Constants mirroring net opcodes (as `nc.rs:13-17` does):
   ```rust
   const NET_ENDPOINT: usize = 5;
   const SOCKET_TCP: u8 = 0x10;
   const CONNECT: u8 = 0x12;
   const SEND_OP: u8 = 0x13;
   const RECV_OP: u8 = 0x14;
   const CLOSE: u8 = 0x15;
   const SOCKET_STATE: u8 = 0x19; // from Phase 1
   ```

3. `main()`:
   - Read argv via `sys_spawn_args(&mut arg_buf)` (`arg_buf: [u8;128]`), as
     `nc.rs:29-38`. Empty → print `Usage: curl http://IP[:PORT][/path]`, return.
   - `parse_url(args_str)` → `(host: &str, port: u16, path: &str)`. On error,
     print `curl: invalid URL`, return.
   - `parse_ipv4(host)` (lift verbatim from `nc.rs:153-161`). On error → print
     `curl: invalid host`, return.

4. SOCKET_TCP → cap_id (copy `nc.rs:59-69`).

5. CONNECT (copy `nc.rs:71-86`); on failure print `curl: connect failed`,
   `close_socket(cap)`, return. Print `connecting...`.

6. Build the HTTP request bytes into a `[u8; 400]` with a cursor:
   `GET <path> HTTP/1.0\r\nHost: <host>\r\nConnection: close\r\n\r\n`.
   **Before** writing bytes, validate the total would fit: if
   `9 + path.len() + host.len() + 50 > 400` (50 = fixed header overhead),
   print `curl: URL too long` and return.
   Then build SEND message `[SEND_OP][cap:8][request...]` and run the
   retry-until-buffered loop (copy structure from `nc.rs:100-111`, but break
   when `n >= request_len`).

7. Response accumulation loop (≤ 500 iterations).
   **`sys_recv` returns sender ID, not byte count** (confirmed `task.rs:673`).
   Zero the buffer before each call; scan for last non-zero to find data end:
   ```rust
   let mut response: Vec<u8> = Vec::new();
   let mut recv_msg = [0u8; 13];
   recv_msg[0] = RECV_OP;
   recv_msg[1..9].copy_from_slice(&cap_id.to_le_bytes());
   recv_msg[9..13].copy_from_slice(&2048u32.to_le_bytes());

   for _ in 0..500 {
       let mut buf = [0u8; 2048]; // zeroed — detection relies on this
       sys_send(NET_ENDPOINT, &recv_msg);
       match sys_recv(0, &mut buf) {
           SyscallResult::Ok(_) => {
               // sys_recv returns sender_id, NOT byte count.
               // Detect received bytes by scanning for last non-zero.
               let n = buf.iter().rposition(|&b| b != 0).map(|i| i + 1).unwrap_or(0);
               if n > 0 {
                   response.extend_from_slice(&buf[..n]);
               } else {
                   // 0-byte RECV: check state to distinguish "no data yet"
                   // from "server sent FIN and buffer is drained".
                   let st = query_state(cap_id);
                   if st == 0x06 || st == 0x00 {
                       // CloseWait/Closed: drain one more time to catch any
                       // data that arrived alongside the FIN packet.
                       let mut final_buf = [0u8; 2048];
                       sys_send(NET_ENDPOINT, &recv_msg);
                       if let SyscallResult::Ok(_) = sys_recv(0, &mut final_buf) {
                           let fn_ = final_buf.iter()
                               .rposition(|&b| b != 0).map(|i| i + 1).unwrap_or(0);
                           if fn_ > 0 { response.extend_from_slice(&final_buf[..fn_]); }
                       }
                       break;
                   }
                   sys_yield();
               }
           }
           _ => break,
       }
   }
   ```

8. Parse + print: find first `\r\n\r\n` in `response` via
   `response.windows(4).position(|w| w == b"\r\n\r\n")`. Print the status line
   (bytes up to first `\r\n`) so the test matches `200`; then print the body
   (bytes after the `\r\n\r\n`). Print as UTF-8 lossily (`core::str::from_utf8`,
   on error print raw byte-by-byte or a notice).

9. `close_socket(cap_id)` (copy `nc.rs:143-150`).

10. Add helper `query_state(cap) -> u8`: send `[SOCKET_STATE][cap:8]`, recv 1
    byte, return it (default `0x00` on recv error).

11. Verify:
    ```
    cargo check -p app-net-tools --target riscv64gc-unknown-none-elf
    ```

## Todo List

- [ ] Header + imports + opcode constants.
- [ ] `parse_url` helper (host/port/path).
- [ ] Lift `parse_ipv4` / `parse_u16` from `nc.rs`.
- [ ] SOCKET_TCP → CONNECT → print "connecting...".
- [ ] Build GET request, retry-SEND until buffered.
- [ ] Use zero-scan RECV pattern (`rposition`); do NOT use `Ok(n)` as byte count.
- [ ] Response accumulation loop with SOCKET_STATE done-detection + 500 cap.
- [ ] Split headers/body on `\r\n\r\n`, print status line + body.
- [ ] CLOSE on all exit paths.
- [ ] `cargo check -p app-net-tools` clean.

## Success Criteria

- `cargo check -p app-net-tools --target riscv64gc-unknown-none-elf` exits 0.
- Run from shell: `curl http://10.0.2.2:<port>/` prints a line containing `200`
  and the body (`HELLO` for the test server).
- No infinite loop: with a server that holds the connection open, curl exits
  after ≤ 500 RECV iterations rather than hanging.
- `\r\n\r\n` spanning two RECV chunks is handled (search over full `response`).

## Evidence

**Completion Status**: ✅ Complete (2026-06-03)

**Build Verification**:
```
cargo check -p app-net-tools --target riscv64gc-unknown-none-elf
```
**Result**: Exit 0 (clean).

**Code Changes**:
- Full HTTP/1.0 GET client in `cells/apps/net-tools/src/bin/curl.rs`
- Implemented URL parser: `http://IP[:PORT][/path]` with validation
- SOCKET_TCP → CONNECT → retry-SEND flow with Established detection
- Response accumulation loop with SOCKET_STATE (0x19) done-detection
- Zero-scan RECV pattern (`rposition`) for byte-boundary detection
- `\r\n\r\n` split-across-chunks handling via full buffer scan
- CLOSE on all exit paths (success and error)
- 500-iteration caps on SEND/RECV loops to prevent hangs

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| `sys_recv` byte count confusion — **confirmed**: returns sender ID | N/A | Resolved | Use zero-scan (`rposition`) pattern; documented in step 7 |
| CloseWait + buffered data truncation race | Medium | High | Drain one extra RECV after CloseWait detected (step 7) |
| Server keeps connection open → curl loops forever | Medium | High | 500-iteration cap on RECV/state loop |
| `\r\n\r\n` split across two RECV chunks | Low | Medium | Search full `response` buffer, not per-chunk (step 8) |
| HTTP request > net IPC payload (~503B) → truncated SEND | Low | Medium | Keep request ≤ 400B; only GET + Host + Connection headers |
| `arg_buf` borrowed `&str` dropped before request built | Low | Medium | Build request bytes while `arg_buf` in scope, or copy host to stack array |
| Body is binary (non-UTF8) → print panics | Low | Low | Use `from_utf8` with lossy/raw fallback; test body is ASCII |

## Security Considerations

- `curl` runs as an unprivileged cell; it only gets a socket cap it created. No
  filesystem or other authority touched.
- URL parser must not index out of bounds on malformed input (empty host,
  trailing `:`); all `parse_*` helpers return `Option` and bail on bad input.
- No TLS — plaintext HTTP/1.0 only. Out of scope; document as a limitation.
- **RECV byte-count limitation**: `sys_recv` returns sender ID, not byte count.
  `curl` uses zero-scan to find the data end. This is reliable for ASCII HTTP
  (no embedded zero bytes) but would truncate binary response bodies. Accepted
  for Phase B; a RECV2 opcode with `[n:4 LE][data]` can add proper counting
  in a future phase when binary transfer is needed.

## Next Steps

- Phase 3 builds `/bin/curl` into the disk image and adds the e2e test.
