---
title: "Phase E: Hardening + Reboot Persistence"
description: "Make FAT16 writes survive a reboot; harden SeekFrom overflow + block-syscall sector range."
status: complete
priority: P1
effort: 5h
branch: main
tags: [vfs, fat16, kernel, syscall, persistence, hardening, testing]
created: 2026-06-03
completed: 2026-06-03
---

# Phase E: Hardening + Reboot Persistence

Phase D (commit `34aa7cfe`) added FAT16 write persistence; `vfs_fat16_write_read`
proves same-session write+read. Phase E makes writes survive a reboot and closes
two Phase D code-review findings.

## Goal

1. Harden two safety findings (no behavior change to happy path).
2. Add a clean shutdown path (kernel SBI SRST + shell `shutdown` built-in).
3. Make the test harness wait for QEMU's natural exit so the disk image flushes.
4. Prove via integration test that a FAT16 write survives a full reboot.

## Phases

| # | Phase | Status | Depends on | Files |
|---|-------|--------|-----------|-------|
| 1 | [Hardening (SeekFrom cap + sector-range guard)](phase-01-hardening.md) | complete | — | `block_stream.rs`, `syscall.rs` |
| 2 | [Kernel shutdown syscall + shell built-in](phase-02-shutdown-syscall.md) | complete | — | `syscall.rs`, `ostd/syscall.rs`, `cmd_sys.rs`, `executor.rs` |
| 3 | [QemuRunner graceful exit](phase-03-qemu-graceful-exit.md) | complete | — | `tests/integration/src/lib.rs` |
| 4 | [Reboot persistence integration test](phase-04-reboot-persistence-test.md) | complete | 2, 3 | `tests/integration/tests/boot.rs` |

## Dependency graph

```
Phase 1 ──┐ (independent)
Phase 2 ──┼──┐
Phase 3 ──┘  ├──> Phase 4 (needs shutdown built-in + wait_for_natural_exit)
             │
```

Phases 1, 2, 3 are mutually independent and may proceed in parallel.
Phase 4 is the integration gate — it requires Phases 2 and 3 to land first.

## File ownership (no overlap between parallel phases)

| File | Phase | Conflict risk |
|------|-------|---------------|
| `cells/services/vfs/src/block_stream.rs` | 1 only | none |
| `kernel/src/task/syscall.rs` | **1 and 2** | SEQUENCE: do Phase 1 edit (lines ~1068–1091) then Phase 2 edit (enum ~253, handler before `}` ~1107, numeric map ~1201). Distinct line ranges — apply 1 before 2. |
| `libs/ostd/src/syscall.rs` | 2 only | none |
| `cells/apps/shell/src/cmd_sys.rs` | 2 only | none |
| `cells/apps/shell/src/executor.rs` | 2 only | none |
| `tests/integration/src/lib.rs` | 3 only | none |
| `tests/integration/tests/boot.rs` | 4 only | none |

**Coordination note:** Phases 1 and 2 both touch `syscall.rs`. They edit disjoint
regions but must not run truly concurrently (Edit needs a fresh Read). Run Phase 1's
`syscall.rs` edit, then Phase 2's. No `libs/api/` change is needed (raw-502 pattern).

## Key decisions

- **Raw syscall 502 (NOT a `ViSyscall` enum entry)** — avoids the Law 1 `libs/api`
  2x-confirmation gate. Mirrors the existing BlkRead=500 / BlkWrite=501 precedent
  verified at `kernel/src/task/syscall.rs:1199-1201` and `libs/ostd/src/syscall.rs:63-78`.
- **SBI SRST from S-mode** — kernel issues `ecall` to OpenSBI (M-mode), which powers
  off QEMU. User cells cannot call SBI directly; they go through syscall 502.
- **Graceful exit before reboot** — `child.try_wait()` poll loop ensures QEMU's
  VirtIO block backend flushes `disk_v3.img` before the second boot reads it.

## Success criteria (whole phase)

- `cargo check` clean for: `-p ViCell-kernel -p service-vfs -p ostd -p app-shell --target riscv64gc-unknown-none-elf`
- `cargo check --manifest-path tests/integration/Cargo.toml` clean.
- `cargo test -p ViCell-integration-tests vfs_fat16_reboot_persistence` passes
  (writes `REBOOT_OK`, shuts down, reboots, reads `REBOOT_OK` back).
- Existing `vfs_fat16_write_read` still passes (no regression).
- `shutdown` shell built-in cleanly terminates QEMU within 15s.

## Out of scope (Phase F)

OP_WRITE header widening, FAT16 subdirectories, OP_UNLINK for FAT16, capability
gate for block syscalls, ACPI/PSCI power management, stress/power-loss recovery.
