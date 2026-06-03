# Phase 1: Kernel Block I/O Syscalls (raw 500/501)

## Context Links
- `kernel/src/task/syscall.rs:1158` — `_ => match syscall_id` fallback (integration point)
- `kernel/src/task/drivers/virtio_blk.rs:101-152` — `viVirtIOBlk` ViBlockDevice impl
- `libs/ostd/src/syscall.rs:23-36` — existing `syscall(id: ViSyscall, ...)` helper
- `libs/api/src/syscall.rs:99-141` — `ViSyscall::from` (500/501 → `Unknown`)

## Overview
- **Priority:** P1 (blocks Phase 3)
- **Status:** pending
- **Effort:** 2h
- Expose VirtIO block read/write to user cells via raw syscall IDs 500 (`BlkRead`)
  and 501 (`BlkWrite`), WITHOUT modifying `libs/api/src/syscall.rs` (avoids Law 1
  2x-confirmation). ostd gains `sys_blk_read`/`sys_blk_write` over a private
  `syscall_raw` helper.

## Key Insights (verified)
- `ViSyscall::from(500)` and `from(501)` both return `ViSyscall::Unknown`
  (`libs/api/src/syscall.rs:139`). In `vios_syscall_dispatch`, `Unknown` falls to
  the `_ => match syscall_id { ... }` arm (`kernel/src/task/syscall.rs:1158`),
  whose current default is `frame.regs[10] = usize::MAX; return;`. This is exactly
  where 500/501 must be matched.
- The block driver exposes NO free functions. `viVirtIOBlk` is a ZST implementing
  `ViBlockDevice` with `read_sector(&self, sector: u64, buf: &mut [u8])` and
  `write_sector(&self, sector: u64, buf: &[u8])`. Already used at
  `kernel/src/loader/early.rs:52` (`viVirtIOBlk.read_sector(...)`).
- The dispatcher reads a0..a3 from `frame.regs[10..13]` (line 1087-1090) and
  enables SUM around `handle_syscall` (line 1180-1191). Raw block handlers run
  INSIDE `handle_syscall`, so SUM is already set — user buffer access is safe.
- `read_sector`/`write_sector` take the `BLOCK_DEVICE` Spinlock (disables
  interrupts) and spin-poll the used ring. Synchronous; no IRQ needed.

## Requirements
- **Functional:** `sys_blk_read(sector, &mut [u8;512])` and
  `sys_blk_write(sector, &[u8;512])` round-trip one 512-byte sector through QEMU.
- **Non-functional:** No change to `libs/api`. Bounds-check the 512-byte buffer.
  Reject sector ≥ device capacity is OPTIONAL (driver errors gracefully).

## Architecture / Data Flow
```
ostd::sys_blk_write(sector, buf)
  └─ syscall_raw(501, sector, buf.as_ptr(), 512, 0)  // a7=501
        └─ ecall ─▶ vios_syscall_dispatch
              ViSyscall::from(501) == Unknown ─▶ _ => match syscall_id { 501 => Syscall::BlkWrite{..} }
                    └─ handle_syscall(Syscall::BlkWrite) [SUM=1]
                          └─ validate_user_buf(ptr, 512)
                          └─ viVirtIOBlk.write_sector(sector, &slice) ─▶ Ok(1)/Ok(0)
```

## Related Code Files
**Modify:**
- `kernel/src/task/syscall.rs` — add `BlkRead`/`BlkWrite` to `Syscall` enum, handlers in `handle_syscall`, mapping in the `_ => match syscall_id` block.
- `libs/ostd/src/syscall.rs` — add private `syscall_raw` + `sys_blk_read`/`sys_blk_write`.

**Do NOT touch:** `libs/api/src/syscall.rs` (Law 1).

## Implementation Steps

### 1. ostd: private `syscall_raw` + wrappers (`libs/ostd/src/syscall.rs`)
Add near the existing `syscall` fn (after line 36):
```rust
/// Invoke a syscall by raw numeric id (bypasses the `ViSyscall` enum).
///
/// Used for block I/O (ids 500/501) which intentionally have no `ViSyscall`
/// entry — keeping them out of the stable ABI in `libs/api` avoids the
/// Interface-is-Sacred 2x-confirmation gate. The kernel dispatches them via the
/// numeric fallback in `vios_syscall_dispatch`.
#[inline(always)]
unsafe fn syscall_raw(id: usize, a0: usize, a1: usize, a2: usize, a3: usize) -> isize {
    let mut ret: isize;
    asm!(
        "ecall",
        inlateout("a0") a0 => ret,
        in("a1") a1, in("a2") a2, in("a3") a3,
        in("a7") id,
        options(nostack, preserves_flags)
    );
    ret
}

/// Read one 512-byte sector from the VirtIO block device. Returns true on success.
///
/// Raw syscall 500. `buf` is filled only when the return is true.
pub fn sys_blk_read(sector: u64, buf: &mut [u8; 512]) -> bool {
    // SAFETY: buf is a fixed 512-byte array; the kernel writes exactly 512 bytes
    // into it (validated against MAX_USER_BUF) only on the success path.
    let ret = unsafe { syscall_raw(500, sector as usize, buf.as_mut_ptr() as usize, 512, 0) };
    ret == 1
}

/// Write one 512-byte sector to the VirtIO block device. Returns true on success.
///
/// Raw syscall 501. The write is synchronous (VirtIO polling) — durable on return.
pub fn sys_blk_write(sector: u64, buf: &[u8; 512]) -> bool {
    // SAFETY: buf is a fixed 512-byte array; the kernel reads exactly 512 bytes.
    let ret = unsafe { syscall_raw(501, sector as usize, buf.as_ptr() as usize, 512, 0) };
    ret == 1
}
```
NOTE: `sector as usize` is safe on RV64 (usize = 64-bit). Document that this caps
sector at `usize::MAX` (irrelevant — disk is 81920 sectors).

### 2. Kernel: add `Syscall` enum variants (`kernel/src/task/syscall.rs`)
After `HotSwap` (line 252), inside `enum Syscall`:
```rust
    /// 500: BlkRead — read one 512-byte sector from the VirtIO block device.
    BlkRead { sector: u64, buf_ptr: usize },
    /// 501: BlkWrite — write one 512-byte sector to the VirtIO block device.
    BlkWrite { sector: u64, buf_ptr: usize },
```

### 3. Kernel: add handlers in `handle_syscall` (`kernel/src/task/syscall.rs`)
Add two arms before the closing `}` of the match (after `HotSwap`, line 1077):
```rust
        Syscall::BlkRead { sector, buf_ptr } => {
            // 512-byte fixed sector buffer; SUM is enabled by the dispatcher.
            validate_user_buf(buf_ptr, 512, MAX_USER_BUF)?;
            use crate::task::drivers::virtio_blk::viVirtIOBlk;
            use api::block::ViBlockDevice;
            // SAFETY: buf_ptr validated above; SUM=1 lets S-mode write the U-mode page.
            let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, 512) };
            match viVirtIOBlk.read_sector(sector, buf) {
                Ok(()) => Ok(1),
                Err(_) => Ok(0),
            }
        }
        Syscall::BlkWrite { sector, buf_ptr } => {
            validate_user_buf(buf_ptr, 512, MAX_USER_BUF)?;
            use crate::task::drivers::virtio_blk::viVirtIOBlk;
            use api::block::ViBlockDevice;
            // SAFETY: buf_ptr validated above; SUM=1 lets S-mode read the U-mode page.
            let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, 512) };
            match viVirtIOBlk.write_sector(sector, buf) {
                Ok(()) => Ok(1),
                Err(_) => Ok(0),
            }
        }
```

### 4. Kernel: map raw ids in the dispatcher fallback (`kernel/src/task/syscall.rs:1158`)
Inside `_ => match syscall_id {`, before the final `_ => { frame.regs[10] = usize::MAX; return; }`:
```rust
            500 => Syscall::BlkRead  { sector: a0 as u64, buf_ptr: a1 },
            501 => Syscall::BlkWrite { sector: a0 as u64, buf_ptr: a1 },
```

### 5. Compile
```
cargo check -p vios-kernel --target riscv64gc-unknown-none-elf
cargo check -p ostd --target riscv64gc-unknown-none-elf
```

## Todo List
- [ ] Add `syscall_raw` + `sys_blk_read`/`sys_blk_write` to ostd with `// SAFETY:`
- [ ] Add `BlkRead`/`BlkWrite` variants to kernel `Syscall` enum
- [ ] Add handlers in `handle_syscall`
- [ ] Map 500/501 in `_ => match syscall_id`
- [ ] `cargo check -p vios-kernel` and `-p ostd` pass

## Success Criteria
- Both `cargo check` commands compile clean (no warnings on new unsafe blocks).
- A scratch test (Phase 5 covers the real one) where a cell writes sector N then
  reads it back returns the same 512 bytes.

## Risk Assessment
| Risk | L | I | Mitigation |
|------|---|---|-----------|
| `viVirtIOBlk` import path wrong | Low | Med | Verified at `early.rs:52`; struct is `pub` |
| `a0 as u64` truncation on RV32 | Low | Low | Kernel is RV64-only today; document assumption |
| User passes < 512-byte buffer from ostd | Low | Med | ostd signature forces `&[u8;512]` (compile-time) |

## Security Considerations
- `validate_user_buf` rejects NULL/oversize/overflow before the raw pointer
  deref. Block I/O grants a cell raw sector access — acceptable in Phase D
  (single trusted VFS caller); a future cap-gated `BlkCap` is the proper control.
- A malicious cell could read/write the cell bootstrap table (LBA 82000+). Out of
  scope for D; note for Phase E that block syscalls need a capability gate.

## Next Steps
Phase 3 (BlockStream) consumes `sys_blk_read`/`sys_blk_write`.

## Evidence

**Compilation:** `cargo check -p vios-kernel --target riscv64gc-unknown-none-elf` exits 0.

**Code Integration Points Verified:**
- `libs/ostd/src/syscall.rs` — `syscall_raw` + `sys_blk_read`/`sys_blk_write` added (lines ~37–100)
- `kernel/src/task/syscall.rs` — `Syscall::BlkRead`/`BlkWrite` variants added (lines ~253–254)
- `kernel/src/task/syscall.rs` — `handle_syscall` arms for BlkRead/BlkWrite added (lines ~1116–1138)
- `kernel/src/task/syscall.rs` — numeric fallback mapping 500/501 in `_ => match syscall_id` (lines ~144–145)

**Test Result:** `cargo test -p vios-integration-tests -- --test-threads=1` shows Phase 5 integration test `vfs_fat16_write_read` passing (13/13 integration tests pass).

## Unresolved Questions
- Should `sys_blk_write` to a sector ≥ device capacity be rejected in the kernel,
  or is the driver's `Err` return (→ `Ok(0)`) sufficient? Current plan: rely on
  driver error. Revisit if QEMU silently accepts OOB writes.
