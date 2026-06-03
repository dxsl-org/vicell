---
title: "Phase A — Network TCP Data-Path + Integration Tests"
description: "Wire TCP CONNECT/SEND/RECV in net service; add loopback integration test. Closes the last major gap before IoT story is credible."
status: pending
priority: P1
effort: "2-3 sessions"
branch: main
tags: [network, tcp, integration-tests, iot]
blockedBy: []
created: 2026-06-03
---

# Phase A: Network TCP Data-Path + Integration Tests

## Overview

DHCP is working (10.0.2.15 verified). The VirtIO NIC kernel driver is complete.
smoltcp is integrated. The only missing piece: TCP socket data operations
(CONNECT/SEND/RECV/LISTEN/ACCEPT) return `0xFF` stub in the net Cell.

This plan wires the TCP data-path and adds a loopback integration test to
prove it end-to-end. MicroPython REPL is already verified (Phase 18 test passes)
— no work needed there.

## Scope

| Task | Status | Notes |
|------|--------|-------|
| Wire TCP CONNECT | ❌ Stub | `main.rs:197-202` |
| Wire TCP SEND | ❌ Stub | `main.rs:197-202` |
| Wire TCP RECV | ❌ Stub | `main.rs:197-202` |
| Add socket state tracking | ❌ Missing | Prevent double-CONNECT etc. |
| Integration test: TCP loopback | ❌ Missing | CONNECT→SEND→RECV→CLOSE |
| MicroPython verify | ✅ Done | Phase 18 test already passes |

## Phases

| Phase | Name | Status |
|-------|------|--------|
| 1 | [Wire TCP Data-Path](./phase-01-tcp-data-path.md) | Pending |
| 2 | [TCP Integration Test](./phase-02-tcp-integration-test.md) | Pending |

## Key Context

- **Net Cell entrypoint**: `cells/services/net/src/main.rs`
- **Stub location**: `handle_socket_syscall()` lines 162-208
- **Socket table**: `cells/services/net/src/socket_table.rs` — CapId→SocketHandle, `get()` ready
- **smoltcp features**: socket-tcp ✅, 4096B RX+TX buffers already allocated per socket
- **Test harness**: `tests/integration/src/lib.rs` — QemuRunner with `wait_for()` + `send_line()`
- **Existing tests**: `tests/integration/tests/boot.rs` — 9 tests, DHCP already passes
- **QEMU network**: user-mode SLIRP, gateway 10.0.2.2, DNS 10.0.2.3

## Dependencies

- smoltcp 0.11 `socket-tcp` feature: ✅ already in Cargo.toml
- VirtIO NIC driver: ✅ complete (`kernel/src/task/drivers/virtio_net.rs`)
- Kernel NetTx/NetRx syscalls: ✅ complete (`syscall.rs:1021-1036`)
- ostd sys_net_tx/rx wrappers: ✅ complete (`libs/ostd/src/syscall.rs:461-481`)
