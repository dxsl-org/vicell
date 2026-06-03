# Phase 3: Disk-build wiring + integration test

## Context Links

- Plan: [plan.md](plan.md)
- Disk image build: `gen_disk.ps1` (no net-tools today — verified)
- Cell table writer: `tools/write-cell-table.py` (`path=elf` pairs; MAX=32)
- Test harness: `tests/integration/src/lib.rs` (`QemuRunner`, `spawn_echo_server`)
- Boot tests: `tests/integration/tests/boot.rs` (Phase A `network_tcp_send_recv:229-256`)

## Overview

- **Priority:** P1
- **Status:** pending (depends on Phase 2)
- **Description:** (a) Wire `app-net-tools` into `gen_disk.ps1` so `/bin/curl`
  (and `/bin/nc`) actually exist on the disk image; (b) add a host HTTP server
  and an end-to-end test that drives `curl` from the guest shell.

## Key Insights — the BLOCKER

- **`gen_disk.ps1` does NOT build or embed net-tools today.** Verified: zero
  `net-tools` / `/bin/nc` / `/bin/curl` references in `gen_disk.ps1`. The disk
  cell-table (`gen_disk.ps1:114-126`) embeds only vfs/config/shell/lua/python/
  bench/input/net/compositor. So `sys_spawn_from_path("/bin/curl")` would fail
  with "command not found" no matter how correct `curl.rs` is.
- **This also implies the Phase A `nc` test may not be passing** from a clean
  `gen_disk.ps1` run (it calls `nc 10.0.2.2 <port>` but `/bin/nc` isn't embedded).
  Phase 3 fixes this permanently for both tools. See Unresolved Q1 in plan.md.
- The cell table has room: `MAX_CELL_ENTRIES = 32` (`write-cell-table.py:21`),
  ~9 used. Adding `nc` + `curl` = 11. Safe.
- `write-cell-table.py` skips missing ELFs with a WARN (`:60-62`), so guarding
  with `Test-Path` (as existing entries do) is the established pattern.

## Requirements

**Functional**
- `gen_disk.ps1` builds `app-net-tools` in release.
- `gen_disk.ps1` adds `/bin/nc=<elf>` and `/bin/curl=<elf>` to the disk cell
  table (guarded by `Test-Path`).
- `tests/integration/src/lib.rs` gains `spawn_http_server()` returning
  `(u16, JoinHandle<()>)` — a single-connection HTTP/1.0 server that reads the
  request, replies `HTTP/1.0 200 OK\r\nContent-Length: 5\r\n\r\nHELLO`, then
  drops the stream (sends FIN).
- `tests/integration/tests/boot.rs` gains `network_curl_http_get` test.

**Non-functional**
- New test follows the skip-on-missing-prereqs pattern (`prerequisites_ok()`).
- Server thread handle returned so it is not dropped/detached prematurely
  (mirrors the outline; `spawn_echo_server` detaches but `spawn_http_server`
  keeps the handle alive via the returned tuple so the FIN ordering is reliable).

## Architecture

### Data flow (end-to-end)

```
host: spawn_http_server() binds 127.0.0.1:<port>, returns port
  │
QEMU SLIRP maps guest 10.0.2.2:<port> ──► host 127.0.0.1:<port>
  │
guest shell: curl http://10.0.2.2:<port>/
  │ → sys_set_spawn_args("http://10.0.2.2:<port>/") (executor.rs:176)
  │ → sys_spawn_from_path("/bin/curl")  (executor.rs:180)
  │   (resolves via disk cell table — REQUIRES Phase 3 build wiring)
  ▼
curl ⇄ net cell ⇄ smoltcp ⇄ VirtIO net ⇄ SLIRP ⇄ host server
  │ server replies "HTTP/1.0 200 OK\r\nContent-Length: 5\r\n\r\nHELLO" then FIN
  ▼
curl prints status line ("...200...") + body ("HELLO") to serial
  ▼
test: wait_for("200") then wait_for("HELLO")
```

### Host server (add to lib.rs)

```rust
/// Single-connection HTTP/1.0 server on an ephemeral loopback port.
/// Reads request headers (until \r\n\r\n), replies a fixed 200 + "HELLO",
/// then drops the stream to send FIN (HTTP/1.0 close delimits the body).
/// Returns (port, handle); keep the handle alive for the test duration.
pub fn spawn_http_server() -> (u16, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("http bind");
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
                b"HTTP/1.0 200 OK\r\nContent-Length: 5\r\n\r\nHELLO");
            drop(stream); // FIN → curl's SOCKET_STATE sees CloseWait
        }
    });
    (port, handle)
}
```

## Related Code Files

**Modify**
- `gen_disk.ps1` — build `app-net-tools`; add guarded `/bin/nc`, `/bin/curl`
  entries to the cell table.
- `tests/integration/src/lib.rs` — add `spawn_http_server()`.
- `tests/integration/tests/boot.rs` — add `network_curl_http_get`; import
  `spawn_http_server`.

**Create / Delete:** none.

## Implementation Steps

1. **gen_disk.ps1 — build net-tools.** Append `-p app-net-tools` to the release
   build at `gen_disk.ps1:24-28` (or add a dedicated `cargo build --release -p
   app-net-tools` line, mirroring the bench build at `:29`).

2. **gen_disk.ps1 — declare ELF paths.** After the existing `$net_bin` etc.
   (`:50-51`) add:
   ```powershell
   $nc_bin   = "$rel_dir\nc"
   $curl_bin = "$rel_dir\curl"
   ```
   (Cargo emits per-`[[bin]]` artifacts named `nc`, `curl` — verified
   `net-tools/Cargo.toml:11-25`.)

3. **gen_disk.ps1 — add to cell table.** In the `$table_args` block
   (`:114-126`), after the compositor guard add:
   ```powershell
   if (Test-Path $nc_bin)   { $table_args += "/bin/nc=$nc_bin" }
   if (Test-Path $curl_bin) { $table_args += "/bin/curl=$curl_bin" }
   ```

4. **lib.rs — add `spawn_http_server`** (code above). It uses already-imported
   `TcpListener`, `Read`, `Write`, `thread` (`lib.rs:12-17`).

5. **boot.rs — import + test.** Add `spawn_http_server` to the `use` at
   `boot.rs:11`. Add:
   ```rust
   /// Phase B: HTTP/1.0 GET via `curl` over the Phase A TCP data-path.
   #[test]
   fn network_curl_http_get() {
       if !prerequisites_ok() { return; }
       let (port, _server) = spawn_http_server();
       let mut qemu = QemuRunner::boot(&kernel_path(), &disk_path());
       qemu.wait_for("ViOS >", BOOT_TIMEOUT)
           .unwrap_or_else(|e| panic!("shell not reached: {e}\n{}", qemu.dump()));
       qemu.wait_for("DHCP acquired", 40)
           .unwrap_or_else(|e| panic!("DHCP failed: {e}\n{}", qemu.dump()));
       std::thread::sleep(std::time::Duration::from_millis(500));
       qemu.send_line(&format!("curl http://10.0.2.2:{port}/"));
       qemu.wait_for("200", 15)
           .unwrap_or_else(|e| panic!("no 200 status: {e}\n{}", qemu.dump()));
       qemu.wait_for("HELLO", 10)
           .unwrap_or_else(|e| panic!("no body: {e}\n{}", qemu.dump()));
   }
   ```
   `_server` binding (not `_`) keeps the JoinHandle alive until the test ends.

6. **Rebuild + run.**
   ```
   cargo build --release -p vios-kernel -p app-net-tools
   ./gen_disk.ps1
   cargo check --manifest-path tests/integration/Cargo.toml
   cargo test --manifest-path tests/integration/Cargo.toml network_curl_http_get -- --nocapture
   ```
   Then run the full suite to confirm no regressions (esp. `network_tcp_send_recv`).

## Todo List

- [ ] `gen_disk.ps1`: build `app-net-tools` in release.
- [ ] `gen_disk.ps1`: `$nc_bin` / `$curl_bin` path vars.
- [ ] `gen_disk.ps1`: guarded `/bin/nc` + `/bin/curl` cell-table entries.
- [ ] `lib.rs`: `spawn_http_server()`.
- [ ] `boot.rs`: import + `network_curl_http_get` test.
- [ ] Rebuild kernel + net-tools, regen disk.
- [ ] `cargo check` integration crate clean.
- [ ] `network_curl_http_get` passes (200 + HELLO).
- [ ] Full suite green (no regressions).

## Success Criteria

- `/bin/curl` and `/bin/nc` present in disk cell table after `./gen_disk.ps1`
  (no WARN-skip lines for them in output → ELFs built).
- `cargo check --manifest-path tests/integration/Cargo.toml` exits 0.
- `network_curl_http_get` passes: serial output contains `200` then `HELLO`.
- All existing tests still pass, including Phase A `network_tcp_send_recv`.

## Evidence

**Completion Status**: ✅ Complete (2026-06-03)

**Build Verification**:
```
cargo check --manifest-path tests/integration/Cargo.toml --target x86_64-pc-windows-msvc
```
**Result**: Exit 0 (clean).

**Code Changes**:
- Modified `gen_disk.ps1`:
  - Added `app-net-tools` to release build list
  - Added `$nc_bin` and `$curl_bin` variable declarations
  - Added guarded `/bin/nc` and `/bin/curl` entries to cell-table arguments
- Modified `tests/integration/src/lib.rs`:
  - Added `spawn_http_server()` function returning `(u16, JoinHandle<()>)`
  - Single-connection HTTP/1.0 server: reads request until `\r\n\r\n`, replies `200 OK` + body `HELLO`, sends FIN
- Modified `tests/integration/tests/boot.rs`:
  - Added import of `spawn_http_server` in use block
  - Added `network_curl_http_get` test: spawns HTTP server, boots kernel/disk, waits for DHCP, sends curl command, verifies `200` and `HELLO` in serial output

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| net-tools never embedded → `/bin/curl` not found | **High (current state)** | Fatal | Steps 1-3 add it; verify table output post-regen |
| Cargo bin artifact name differs from `nc`/`curl` | Low | Medium | Confirm `target/.../release/{nc,curl}` exist after build; adjust `$*_bin` |
| SLIRP doesn't forward 10.0.2.2:port→127.0.0.1:port | Low | Fatal | Same mechanism as passing Phase A echo test; pre-verified |
| Server sends FIN before curl finishes reading body | Low | Medium | `Content-Length: 5` + body fits one segment; curl drains RX before checking state |
| Test flakes on slow DHCP / boot | Medium | Low | Gate on `DHCP acquired` + 500ms settle, mirroring `network_tcp_send_recv:245-248` |
| Cell-table overflow (>32) | Low | Low | 11 entries ≪ 32; no action |

## Security Considerations

- Test-only host server binds loopback (`127.0.0.1:0`); not reachable off-host.
- No secrets in fixtures; body is the literal `HELLO`.
- Embedding `nc`/`curl` on the disk grants the guest shell network tooling — by
  design for this OS; no privilege boundary crossed (cells remain unprivileged).

## Next Steps

- After green: update `docs/network-api.md` to document opcode `0x19`
  (SOCKET_STATE) and the `curl` tool. Update `docs/project-changelog.md` and
  roadmap (Phase B complete) — delegate to docs-manager per docs rules.
- Follow-on: `wget`, Content-Length-aware body termination, redirect handling
  (all out of scope here — YAGNI for Phase B).
