---
name: project-virtio-bugs
description: Root cause analysis of ViCell VirtIO block hang and keyboard deadlock bugs
metadata:
  type: project
---

## Keyboard Deadlock Root Cause (Phase 05 — FIXED)

**Symptom:** Shell reads first keystroke, then deadlocks.

**Root cause:** The kernel Spinlock (`kernel/src/sync.rs`) disables interrupts on lock + restores on unlock.
After `CONSOLE.lock()` is released (from `file_read`), a pending VirtIO input IRQ fires.
`vi_handle_virtio_irq` (`virtio_blk.rs`) only dispatches to the block device — it never calls
`ack_interrupt()` on the input device. The device's `InterruptStatus` register stays set.
After `plic_complete()`, the PLIC sees the line still HIGH → re-fires immediately → infinite interrupt storm.

**Fix applied:**
- Added `pub static INPUT_DEVICE_IRQ` in `virtio_input.rs` to track which IRQ slot the keyboard is on
- Added `pub fn ack_irq(irq: u32) -> bool` that calls `driver.input.ack_interrupt()`
- Updated `vi_handle_virtio_irq` to dispatch to both block AND input devices

**Why:** Same pattern applies to any new VirtIO device (GPU, net) — always add `ack_interrupt()` in the IRQ handler.

## VirtIO Block Hang Root Cause (Phase 04 — partial fix)

**Symptom:** VirtIO block `read_blocks` hangs (spins on used ring that never gets populated).

**Root cause identified:** Limine's memory map does NOT include MMIO ranges (UART, VirtIO, PLIC, CLINT).
After `activate_paging()`, accesses to VirtIO MMIO registers at `0x10001000+` would cause load/store page faults.

**Fix applied:** Added explicit identity mappings in `init_kernel_paging` (`kernel/src/memory/paging.rs`):
- CLINT: `0x0200_0000 - 0x0201_0000`
- PLIC: `0x0C00_0000 - 0x1000_0000` (64MB — code review confirmed: ~16K leaf PTEs, ~128KB page-table memory, no OOM risk)
- UART + VirtIO: `0x1000_0000 - 0x1001_0000`

`FALLBACK_MEMORY_MAP` in `kernel/src/boot.rs` no longer contains MMIO entries — single source of truth is paging.rs.

**Remaining:** Full verification requires QEMU testing (Phase 03 boot hang must be fixed first).
**How to apply:** If a new device is added at a non-RAM MMIO address, add it to the MMIO mapping list.
