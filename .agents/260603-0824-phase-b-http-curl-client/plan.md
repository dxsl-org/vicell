---
title: "Phase B: HTTP GET Client (curl)"
description: "HTTP/1.0 GET client cell that fetches a real HTTP response over Phase A's TCP data-path."
status: complete
priority: P2
effort: 5h
branch: main
tags: [network, http, curl, net-cell, smoltcp, integration-test]
created: 2026-06-03
---

# Phase B: HTTP GET Client (`curl`)

Build an HTTP/1.0 GET client (`curl` binary) on top of Phase A's TCP outbound
(CONNECT/SEND/RECV/CLOSE). The client connects to QEMU's host (`10.0.2.2:PORT`),
sends a minimal HTTP/1.0 GET, accumulates the full response in the heap, and
prints the body. A host-side HTTP server validates the round-trip end-to-end.

## Why a new opcode is needed

The net cell's `RECV` returns `0` bytes for two distinct conditions:
1. No data yet (TCP still open, server hasn't sent).
2. Server sent FIN and the RX buffer is drained (response complete).

HTTP/1.0 with `Connection: close` delimits the body by **FIN**, not
Content-Length. `curl` cannot distinguish "wait more" from "done" without
peeking at the TCP state. **Solution:** add a `SOCKET_STATE (0x19)` opcode that
returns the smoltcp `tcp::State` as one byte. On a 0-byte RECV, `curl` queries
SOCKET_STATE; `CloseWait (0x06)` or `Closed (0x00)` means the body is complete.

Verified: `cell_opcodes` (`cells/services/net/src/poll_driver.rs:21-42`) uses
`0x10–0x18` and `0x20`; `0x19` is free.

## Phases

| # | Phase | Status | File |
|---|-------|--------|------|
| 1 | Add `SOCKET_STATE (0x19)` opcode to net cell | complete | [phase-01-socket-state-opcode.md](phase-01-socket-state-opcode.md) |
| 2 | Implement `curl` binary (URL parse → GET → body) | complete | [phase-02-curl-http-client.md](phase-02-curl-http-client.md) |
| 3 | Disk-build wiring + integration test | complete | [phase-03-integration-test.md](phase-03-integration-test.md) |

## Dependency graph

```
Phase 1 (SOCKET_STATE opcode)  ─┐
                                ├─► Phase 2 (curl uses 0x19)
                                ┘
Phase 2 (curl binary built)    ───► Phase 3 (disk-build + e2e test)
```

- Phase 2 depends on Phase 1: `curl` cannot compile its done-detection loop
  until the opcode constant + handler exist and the contract is fixed.
- Phase 3 depends on Phase 2: the e2e test spawns `/bin/curl`, which must be
  built AND embedded in the disk image.

## File ownership (no overlap between parallel work)

- Phase 1 owns: `cells/services/net/src/poll_driver.rs`, `cells/services/net/src/main.rs`
- Phase 2 owns: `cells/apps/net-tools/src/bin/curl.rs`
- Phase 3 owns: `gen_disk.ps1`, `tests/integration/src/lib.rs`, `tests/integration/tests/boot.rs`

No file is touched by two phases. Phases 1 and 2 could be done in parallel by
two engineers if Phase 1's opcode contract (below) is agreed first.

## Frozen IPC contract (the interface both phases code against)

All cell→net messages: `[opcode:1][cap:8 LE][payload:*]` (verified
`poll_driver.rs:63-71`). Replies (verified in `main.rs:184-320`):

| Opcode | Value | Request payload | Reply |
|--------|-------|-----------------|-------|
| SOCKET_TCP | 0x10 | (none; cap field ignored) | `[cap_id:8 LE]` (0 = error) |
| CONNECT | 0x12 | `[addr:4][port:2 LE]` | `[0x00]` ok / `[0x01]` err |
| SEND | 0x13 | raw bytes | `[n:4 LE]` bytes buffered (0 = not ready) |
| RECV | 0x14 | `[buf_len:4 LE]` | raw bytes, 0..min(buf_len,4096). **Note:** `sys_recv` returns sender ID, not byte count — detect data via zero-scan on the receive buffer |
| CLOSE | 0x15 | (none) | `[0x00]` |
| **SOCKET_STATE** | **0x19** | (none) | `[state:1]` (NEW — Phase 1) |

State byte encoding (smoltcp `tcp::State`): `0x00 Closed, 0x01 SynSent,
0x02 SynReceived, 0x03 Established, 0x04 FinWait1, 0x05 FinWait2,
0x06 CloseWait, 0x07 Closing, 0x08 LastAck, 0x09 TimeWait, 0x0A Listen,
0xFF unknown`.

## Key environmental facts (verified against codebase)

- Cell entry point is `#[no_mangle] pub fn main()`, NOT `_start` — see
  `nc.rs:26-27`, `curl.rs:6-7`, net `main.rs:63-64`.
- `NET_ENDPOINT = 5` (init spawn order) — `nc.rs:8-10`.
- `app-net-tools` is `#![no_std]` + depends only on `ostd`/`api`
  (`Cargo.toml:7-9`). `ostd` registers a `#[global_allocator]`
  (`libs/ostd/src/heap.rs:52-53`), so `extern crate alloc;` works in `curl.rs`.
- Shell routes `curl <url>` → `sys_set_spawn_args(<url>)` → spawn `/bin/curl`
  (`executor.rs:149,175-187`). argv is the single URL token.
- Net cell IPC buffer is 512 bytes (`main.rs:85`); per-socket TCP buffers are
  4096 RX/TX (`main.rs:198-199`); RECV can return up to 4096 (`main.rs:296`).
- `DHCP acquired` log string is emitted at `dhcp.rs:59` — the test gate works.

## BLOCKER discovered during verification (must be fixed in Phase 3)

`gen_disk.ps1` does **not** build `app-net-tools` and does **not** embed
`/bin/nc` or `/bin/curl` into either image (verified: no net-tools reference in
`gen_disk.ps1`; only `ls/cat/.../net/compositor` are embedded). The Phase A
`network_tcp_send_recv` test calls `nc 10.0.2.2 <port>` — which means either the
Phase A test is currently skipping/failing on the spawn step, or the disk was
hand-patched. **Phase B cannot run `curl` from the shell until net-tools is added
to `gen_disk.ps1`.** This is folded into Phase 3 as the first task. Cell-table
capacity is fine: `MAX_CELL_ENTRIES=32` (`write-cell-table.py:21`), ~9 used.

## Success criteria (whole plan)

- [ ] `cargo check -p service-net --target riscv64gc-unknown-none-elf` passes (SOCKET_STATE added).
- [ ] `cargo check -p app-net-tools --target riscv64gc-unknown-none-elf` passes (curl implemented).
- [ ] `cargo check --manifest-path tests/integration/Cargo.toml` passes.
- [ ] `gen_disk.ps1` builds and embeds `/bin/curl` (and `/bin/nc`).
- [ ] `network_curl_http_get` passes: `200` and `HELLO` appear in serial output.
- [ ] All existing integration tests still pass (no regressions), incl. Phase A `network_tcp_send_recv`.

## Resolved questions (post-validation)

1. **Phase A test never ran.** `gen_disk.ps1` was not updated after Phase A; the
   `network_tcp_send_recv` test has never executed. Phase 3 fixes `gen_disk.ps1`
   for both `/bin/nc` and `/bin/curl` simultaneously — both tests (`network_tcp_send_recv`
   and `network_curl_http_get`) should run green together after the rebuild.
2. **RECV zero-scan accepted.** The `rposition(|b| b != 0)` approach is intentionally
   ASCII-only. Documented as a known limitation in phase-02. A length-prefix RECV2
   opcode is deferred to a future phase when binary transfer is needed.
3. **Phase C is VFS write (FAT32).** After Phase B proves the HTTP client end-to-end,
   Phase C will implement FAT32 write support so `curl http://... > file.txt` can
   persist responses to disk.
