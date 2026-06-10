---
name: feedback-kernel-patterns
description: Kernel coding patterns confirmed by code review — VirtIO dispatch, MMIO mapping, unsafe comments
metadata:
  type: feedback
---

## Capability Registry — Park/Unpark Pattern for Lock-Safe I/O

When a global lock (`CAP_TABLE`) protects a resource (`Box<dyn ViFile>`) that is then used for I/O, DO NOT hold the lock across the I/O call:

```rust
// WRONG: holds CAP_TABLE lock across file.read() — starves all other caps
let result = { let table = CAP_TABLE.lock(); table.get_file(cap_id).read(buf) };

// CORRECT: park/unpark to release lock during I/O
let mut file = CAP_TABLE.lock().park_file(cap_id, owner)?;  // take Box out
let n = file.read(buf);                                      // I/O outside lock
CAP_TABLE.lock().unpark_file(cap_id, file);                  // put Box back
```

`CapResource::File.file` is `Option<Box<dyn ViFile>>` — `None` while parked, `Some` when idle.

**Why:** A block-device-backed ViFile yields inside `read()`. Holding the global cap-table spinlock while yielding starves every other cell's OpenCap/ReadCap/CloseCap for the entire I/O duration. Code review C2 in Phase 07.
**How to apply:** Any kernel spinlock that protects a resource used for blocking I/O must use the park/unpark pattern. Same principle applies to future network sockets and GPU surfaces.

## Capability Ownership — Always Use task.cell_id, Never caller_id as CellId

In `ViCell_syscall_dispatch`, `caller_id` is the **task ID** (TID), not the **cell ID**. For capability ownership checks, always resolve the cell ID from the scheduler:

```rust
let cell_id = SCHEDULER.lock().as_ref()
    .and_then(|s| s.tasks.get(&caller_id))
    .map(|t| t.cell_id)
    .unwrap_or(CellId(0));
```

**Why:** A cell with multiple threads has multiple TIDs but one CellId. Using TID as CellId breaks cross-thread handle sharing and — worse — if TIDs are ever reused, a new task could fail to inherit the right cell context. All spawned tasks currently use CellId(0), but the types are different for future correctness. Code review C6 in Phase 07.
**How to apply:** Grep `CellId(caller_id` — any such pattern is a bug. Replace with the resolver pattern above.

## Capability Lifecycle — revoke_all_for Must Run on Cell Exit

In `Syscall::Exit`, BEFORE `sched.exit_task(caller_id)`, call:

```rust
CAP_TABLE.lock().revoke_all_for(cell_id);
```

**Why:** Without this, every cap a cell holds at exit is permanently orphaned in the global table. If CellIds are ever reused, a new cell with the same ID inherits the dead cell's capabilities — direct authorization bypass. Code review C5 in Phase 07.
**How to apply:** Any "cell/task terminates" code path must call `revoke_all_for`. The Exit syscall handler in `kernel/src/task/syscall.rs` already does this (added in Phase 07).

## CapId Newtype — Re-export from api, Never Duplicate

The kernel uses `pub use api::cap::CapId` in `kernel/src/cell/cap_registry.rs`. Do NOT create a parallel `type CapId = u64` alias in the kernel.

**Why:** Two separate definitions (even `type` aliases) silently allow values to be confused; type-checking between `api::cap::CapId` and `kernel::cell::CapId` would fail at the boundary. A single canonical definition from the API crate is shared by kernel, ostd, and user cells. Code review L4 in Phase 07.
**How to apply:** When adding new capability types (SocketCap, SurfaceCap), define them in `libs/api/src/cap.rs` and `pub use` them in the kernel. Never define new capability ID types in kernel-only code.

## Lua C Binding — cc Crate RISC-V Compiler Detection

The `cc` crate auto-detects the cross-compiler from the target triple. For `riscv64gc-unknown-none-elf`, it looks for `riscv64-unknown-elf-gcc`, but the ViCell toolchain is `riscv-none-elf-gcc` (xpack). The build.rs must explicitly set the compiler when the env var isn't provided:

```rust
if target.contains("riscv") && std::env::var("CC_riscv64gc_unknown_none_elf").is_err() {
    build.compiler("riscv-none-elf-gcc");
    build.flag("-mabi=lp64d");
}
```

**Why:** Without this, `cargo check -p lua` fails with "ToolNotFound: riscv64-unknown-elf-gcc not found" even though `riscv-none-elf-gcc` is on PATH. Code review Phase 10.
**How to apply:** Any C crate built for RISC-V in ViCell must either set this explicitly or require the caller to export `CC_riscv64gc_unknown_none_elf=riscv-none-elf-gcc`.

## no_std Unit Tests — Cannot Run via Default `cargo test` in RISC-V Workspace

ViCell `.cargo/config.toml` sets `target = "riscv64gc-unknown-none-elf"`. When running `cargo test -p <no_std_crate>`, Rust uses this RISC-V target which has no `libtest` or `libstd`. Even `#![cfg_attr(not(test), no_std)]` does NOT help because `test` cfg is still not set for cross-compiled RISC-V targets.

**Fix options:**
1. Run tests with explicit host target: `cargo test -p types --target x86_64-unknown-linux-gnu`
2. Use a `[[test]]` section in the crate's Cargo.toml with a custom test harness
3. Move unit tests to a separate host-only crate that depends on the no_std crate

**Why:** The RISC-V bare-metal target has no OS services, so the standard test harness cannot run. Phase 11 discovered this when trying to add tests to `libs/types`. The integration harness (`tests/integration/harness.rs`) works because it's a std Rust file that uses `std::process::Command`.
**How to apply:** Do not append `#[cfg(test)]` blocks to no_std crate lib.rs files unless using a custom test framework. Use the integration harness for OS-level tests; for pure logic tests, consider a parallel host-only test crate.

## x86_64 IDT — Must Use `extern "x86-interrupt"` Not `extern "C"`

x86_64 IDT gate handlers MUST use `extern "x86-interrupt"` calling convention, NOT `extern "C"`:
```rust
// WRONG — ret returns into the CPU interrupt frame, causing triple-fault
extern "C" fn handler(vec: u64, ...) { ... }

// CORRECT — LLVM generates iretq + saves all caller-saved registers
extern "x86-interrupt" fn handler(frame: InterruptFrame) { ... }
```

Requires `#![feature(abi_x86_interrupt)]` in the crate root (nightly only).

**Why:** The CPU pushes a return frame (RIP/CS/RFLAGS[/RSP/SS]) on interrupt entry. The handler must return with `iretq` which pops this frame. `extern "C"` ends with `ret`, which jumps into the stacked RIP interpreted as random data — triple-fault within the first timer tick (~10ms). Code review critical #1 in Phase 09.
**How to apply:** `hal/arch/x86/src/lib.rs` has `#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]`. All IDT entries in `idt.rs` use `extern "x86-interrupt"`. Same pattern for any future x86_64 exception handler.

## x86_64 Syscall — KERNEL_GS_BASE Must Be Initialized Before Ring-3

`syscall_entry` (AT&T asm, called on `syscall` instruction) uses `swapgs` then `%gs:0` to load the kernel stack. This only works if `KERNEL_GS_BASE` MSR (0xC0000102) points at a valid per-CPU area:

```rust
// In syscall::init():
let cpu_local_addr = core::ptr::addr_of!(CPU_LOCAL) as u64;
wrmsr(IA32_KERNEL_GSBASE, cpu_local_addr);
```

`CPU_LOCAL` is a `CpuLocal { kernel_rsp: u64, user_rsp: u64 }` struct. `set_kernel_stack` updates `kernel_rsp` before each Ring-3 entry.

**Why:** `swapgs` exchanges `GS_BASE` with `KERNEL_GS_BASE`. If `KERNEL_GS_BASE=0` (default), the first `movq %gs:0, %rsp` loads RSP from address 0 → page fault → triple-fault since the IDT handler also needs GS → nested exception → CPU reset. Code review critical #2 in Phase 09.
**How to apply:** `syscall::init()` already does this. `x86_64.rs::set_kernel_stack()` calls both `gdt::set_kernel_stack` (TSS.rsp0 for hardware interrupts) AND `syscall::set_kernel_stack` (GS area for `syscall` instruction).

## x86_64 LAPIC / PTE Traversal — Precondition: Limine Identity Map Must Be Active

Both `apic::init_lapic()` (accesses 0xFEE0_0000) and `paging.rs` PTE traversal (casts physical PTE address to pointer) assume `phys == virt` — valid only while Limine's identity map is active.

These functions must be called **before** `PageTable::activate()` switches to the kernel's own PML4. After activation, LAPIC MMIO and PTE physical addresses need the HHDM offset applied. Both files have doc-comment preconditions explaining this.

**Why:** Limine identity-maps all physical memory (including MMIO) and the kernel region. After the kernel activates its own PML4, only the mapped ranges are accessible. Accessing 0xFEE0_0000 without a virtual mapping → #PF. This is deferred to full memory manager phase.
**How to apply:** Audit any use of `apic::*` or `paging::*` after paging activation. For post-activate LAPIC access, the caller must map 0xFEE0_0000 explicitly first.

## AArch64 Porting Pitfalls vs RISC-V (Critical Differences)

When porting RV64 syscall/trap code to AArch64, four patterns commonly cause silent bugs:

**1. SVC (ecall) ELR semantics are opposite:**
- RV64: `sepc` points AT the `ecall` instruction → must `sepc += 4` to advance
- AArch64: `elr_el1` points PAST the `svc` instruction → **never** do `elr_el1 += 4`
- Mistake: copying the RISC-V `+= 4` increment silently skips the next user instruction

**2. DAIF mask — use bit 2 (I) not all bits:**
- `daifset/daifclr #0xf` masks D,A,I,F (debug, SError, IRQ, FIQ) simultaneously
- `enable_interrupts()` and `interrupts_enabled()` must target the SAME bit (I = bit 7 in DAIF, but the daifclr immediate is bit 2)
- Use `daifset #2` / `daifclr #2` to match the `mrs daif; & (1<<7)` query

**3. `msr` to system registers CANNOT take immediates:**
- `msr cntp_ctl_el0, #1` — assembler error, not legal AArch64
- Must: `mov x9, #1; msr cntp_ctl_el0, x9`
- Affects all timer/GIC/MMU enable writes that take a constant

**4. DTB pointer at entry is in x0, which BSS-clear clobbers:**
- QEMU passes DTB physical address in x0 at `_start`
- BSS-clear loop reuses x0 and x1 → stash to callee-saved (x19) before the loop

**Why:** Code review found all four as BLOCKER-class bugs in Phase 08. They build clean (Rust/LLVM doesn't assemble msr-imm in the test target) but would silently misbehave at runtime.
**How to apply:** Any future AArch64 exception handler adapted from RISC-V must audit ELR handling. Any system-register writes must use a register operand, not an immediate.

## AArch64 MMU Activation — TLB Invalidation is Mandatory Before SCTLR

Before enabling the MMU (`orr x9, x9, #1; msr sctlr_el1, x9`), the sequence must include:
```asm
tlbi vmalle1      // invalidate all EL1 TLB entries
dsb nsh           // inner-shareable domain barrier
isb               // instruction barrier
```

**Why:** On real ARMv8-A hardware (unlike QEMU), the TLB may contain predictive or stale entries from pre-boot. Without invalidation, the first instruction fetch or data access after MMU enable can hit a stale entry and fault. QEMU's TLB starts empty so this is silent, but the ARM ARM (DDI 0487 D13.2.118) mandates it.
**How to apply:** See `hal/arch/arm/src/aarch64/paging.rs::PageTable::activate`. Reuse this order for Phase 09 (x86_64 uses CR0/CR3 instead, but the concept of TLB flush before enable is the same).

## Multi-Target HAL — Use cfg(target_arch) Guards, Not Stub Compilation

When a HAL module (e.g., `aarch64.rs`) contains architecture-specific assembly, gate ALL sub-modules at the Rust module level using `#[cfg(target_arch = "...")]`:

```rust
// In aarch64.rs facade:
#[cfg(target_arch = "aarch64")] pub mod trap;   // has AArch64 asm
#[cfg(target_arch = "aarch64")] pub mod paging; // has AArch64 asm
// separate stub Arch impl for non-aarch64:
#[cfg(not(target_arch = "aarch64"))]
impl Arch for AArch64Arch { ... /* no asm */ }
```

**Why:** Without this, `pub mod trap;` in `aarch64.rs` compiles the AArch64 assembly for RISC-V builds, causing "invalid register x9" assembler errors. The pattern was missing initially and broke the workspace check. Confirmed fix: both `cargo check --workspace` (rv64) and `cargo check -p hal-arm --target aarch64-unknown-none` exit 0.
**How to apply:** Every new arch HAL (Phase 09 x86_64) must follow the same facade pattern: full impl gated on target, stub impl for all others.

## Disk Bootstrap Layout — Append AFTER FAT32, Same Drive

The cell bootstrap table is appended after the FAT32 filesystem at a fixed LBA offset (`CELL_TABLE_BASE_LBA = 82_000`), not inside a partition, not on a separate QEMU drive. `CellTableHeader` and `CellEntry` are both exactly 512 bytes (`const _: () = assert!(size_of::<T>() == 512)`).

**Why:** FAT32 starts at LBA 0 and the filesystem doesn't claim sectors beyond its data area. Writing at LBA 82000 (beyond a 40 MB FAT32 image) is safe without corrupting the filesystem. One QEMU drive is simpler than two. The compile-time size assertion catches mismatches between kernel structs and the Python builder before runtime.
**How to apply:** `CELL_TABLE_BASE_LBA` is defined in `kernel/src/loader/disk_layout.rs`. When adding a new bootstrap cell, add its entry via `gen_disk.ps1` → `tools/write-cell-table.py`. Both kernel and Python must agree on struct layout (432-byte _pad in CellEntry).

## Lock-Snapshot Pattern — Never Hold Two Locks Across Blocking I/O

When reading from a lock-protected table and then doing blocking I/O (e.g., sector reads):
1. Lock the table, snapshot the entry fields into local variables (e.g., `data_lba`, `size`)
2. **Drop the lock**
3. Do the blocking I/O with only the I/O lock held

**Why:** Holding `CELL_TABLE.lock()` across `BLOCK_DEVICE.lock()` with IRQs disabled starves preemption for the entire read (100s of sectors = seconds). Any future code that holds `BLOCK_DEVICE` and then wants `CELL_TABLE` would deadlock. Code review flagged this as C1 in Phase 06. Verified fix: `early.rs::read_file` now snapshots `(data_lba, size)` before dropping the guard.
**How to apply:** Scan for any pattern where a kernel lock is held across a `viVirtIOBlk.read_sector()` / `write_sector()` call — this is always wrong.

## Unaligned Struct Access in ELF Parsers — Use read_unaligned

When casting a slice of bytes to a struct that has `u64` fields (alignment > 1), always use `core::ptr::read_unaligned`:
```rust
let entry: Rela64 = unsafe {
    core::ptr::read_unaligned(slice.as_ptr().add(offset) as *const Rela64)
};
```

**Why:** ELF buffers are `Vec<u8>` with alignment 1. `Rela64` (containing `u64` fields) requires alignment 8. A direct `&*(ptr as *const Rela64)` is UB on RISC-V if the pointer is not 8-byte aligned. LLVM can emit aligned loads and the CPU will trap. Code review flagged this as H3 in Phase 06.
**How to apply:** Any `repr(C)` struct cast from a `&[u8]` slice must use `read_unaligned`. This applies to ELF program headers, section headers, relocations, symbol tables, etc.

## R_RISCV_64 with sym_index=0 — Treat as R_RISCV_RELATIVE

In `kernel/src/loader/reloc.rs`, `R_RISCV_64` with `sym_index == 0` uses the same formula as `R_RISCV_RELATIVE`: `*ptr = base + addend`.

**Why:** LLVM emits `R_RISCV_64` for absolute pointer fixups in PIE cells (vtables, Arc<dyn> data). When `sym_index == 0`, the addend already encodes the original absolute address and only needs rebasing. Returning `NotSupported` for this case would break any cell using `Arc<dyn ViDriver>` or similar — which is most cells (Law 7 pattern). Non-zero `sym_index` is a genuine dynamic symbol reference and is correctly rejected with `NotSupported`.
**How to apply:** See `reloc.rs`. Always check `sym_index` in the relocation info before deciding how to handle `R_RISCV_64`.

## Driver Debugging Methodology — Check Interrupts Before Async

When a kernel driver appears to "deadlock after first event" (input, block, network):
1. **First:** check if the device IRQ is properly acknowledged (`ack_interrupt()` called and PLIC completed)
2. **Second:** check if a Spinlock releases mid-handler and re-enables an unacknowledged IRQ
3. **Only if both pass:** investigate async waker ordering, `sys_recv` blocking semantics, or executor re-poll

**Why:** An interrupt storm (missing `ack_interrupt()`) is a hardware-level event that preempts ALL software execution. No amount of correct async waker ordering fixes a CPU stuck in a trap handler loop. Phase 05 lesson: the plan listed three async root causes, but the actual cause was an IRQ storm — a purely hardware concern that was simpler and more fundamental.
**How to apply:** When debugging any "first event works, second is lost" symptom in ViCell, open `hal/arch/riscv/src/rv64/trap.rs` and `vi_handle_virtio_irq` first. Confirm every device that fires IRQs 1-8 has an `ack_irq()` call registered.

## VirtIO Device IRQ Handler — Always ack every device

When adding a new VirtIO device, add three things:
1. `static DEVICE_IRQ: Spinlock<u32>` initialized to 0; set in `init_driver()` as `(slot as u32) + 1`
2. `pub fn ack_irq(irq: u32) -> bool` that calls `device.ack_interrupt()` and returns whether it matched
3. Call `ack_irq(irq)` from `vi_handle_virtio_irq` in `virtio_blk.rs`

**Why:** VirtIO MMIO `InterruptStatus` stays SET until the driver writes `InterruptAck`. If not cleared, PLIC re-fires the interrupt immediately after `plic_complete`, creating an infinite interrupt storm. This killed the keyboard after the first keystroke (Phase 05 root cause).
**How to apply:** Every new VirtIO device type (GPU Phase 16, NIC Phase 15) needs this pattern before shipping.

## MMIO Mapping — Single Source of Truth in paging.rs

`FALLBACK_MEMORY_MAP` (`kernel/src/boot.rs`) must contain RAM regions only (Bootloader/Kernel/Usable).
MMIO regions (CLINT, PLIC, UART, VirtIO) belong exclusively in the explicit block at the end of `init_kernel_paging` (`kernel/src/memory/paging.rs`).

**Why:** Limine does not report MMIO in its memory map, so the explicit block must run unconditionally for both Limine and fallback paths. Having MMIO in `FALLBACK_MEMORY_MAP` too causes double-mapping (harmless but confusing). Code review flagged this as a medium-priority cleanup.
**How to apply:** If QEMU gains a new device (e.g., a second VirtIO GPU), add its MMIO range to the explicit block in paging.rs, not to the fallback map.

## satp Activation — Mandatory fence + sfence sequence

The `PageTable::activate` function (`hal/arch/riscv/src/rv64/paging.rs`) must issue three operations in a single asm block:
```rust
core::arch::asm!(
    "fence rw, rw",           // ensure all PTE stores are globally visible
    "csrw satp, {satp}",      // activate new page table (SV39, mode=8)
    "sfence.vma zero, zero",  // flush all TLB entries for all ASIDs
    satp = in(reg) satp_val,
    options(nostack),
);
```

**Why:** RISC-V privileged spec §4.3: all PTE stores must complete before SATP write; TLB flush must follow. Split asm blocks risk compiler reordering. The `8usize` literal is also required — `8 << 60` as i32 overflows on 32-bit; infer as usize to be safe.
**How to apply:** If activate() is ever refactored, keep all three instructions in one asm!() block, in this exact order.

## User Task sstatus Convention — Match spawn_from_mem

All user-mode task spawning must set sstatus values that match `spawn_from_mem` convention:
- **trap_frame.sstatus = 0x6020** (SPP=0 U-mode, SPIE=1, FS=Initial)
- **context.sstatus = 0x42120** (SPP=1 S-mode, SPIE=1, SUM=1, FS=Initial)

**Why:** Inconsistent FS bits cause FP trap on first floating-point instruction. Inconsistent SUM means kernel-mode code can't access U-mode pointers (needed for syscall buffer reads). Verified aligned with spawn_from_mem at `kernel/src/task.rs:243,256`.
**How to apply:** Any new spawn path (user_hello, spawn_synthetic, future ELF loaders) must use these exact values.

## User Code Pages — Map R|X|U, Never W

User code pages must be mapped with `VALID | READ | EXECUTE | USER | ACCESSED | DIRTY` — no WRITE bit.

**Why:** W flag on executable pages enables code injection (an attacker who can write to a code page can overwrite instructions). RISC-V PTE semantics: R=1 X=1 W=0 is valid "execute-only" (or read-execute). W=0 ensures even if the user somehow gets a pointer to the code page, they can't modify it.
**How to apply:** See `kernel/src/task/user_hello.rs::spawn()` for the correct flags. Same applies to any future ELF segment mapper for .text sections.

## Exit Syscall — Call exit_task() Before yield_cpu()

In `kernel/src/task/syscall.rs`, the `Syscall::Exit` handler must:
1. Record `task.exit_code`
2. Collect `task.waiters`  
3. Call `sched.exit_task(caller_id)` — moves task to `sched.zombies`
4. Wake waiters
5. Call `super::yield_cpu()`

**Why:** Without `exit_task()`, the task stays in `sched.tasks` as `Terminated` forever — its stacks and code frames are never freed. `pick_next()` already checks `sched.zombies` for the current task's context pointer (verified at `kernel/src/task/scheduler.rs:292`), so moving to zombies before the context switch is safe.
**How to apply:** This pattern is now established in the Exit handler. Any new "task terminates" path must move to zombies before yielding.

## VirtIO HAL `dma_dealloc` — Use vaddr, Not paddr

`VirtioHal::dma_dealloc` (`kernel/src/task/drivers/virtio_hal.rs`) must deallocate using the `vaddr` parameter, not `paddr`:
```rust
unsafe { alloc::alloc::dealloc(vaddr.as_ptr(), layout) };
```

**Why:** Under identity mapping `paddr == vaddr`, so both work today. But if HHDM is ever introduced (VAddr != PAddr), using `paddr` silently deallocates the wrong address, corrupting the allocator. The `Hal` trait provides `vaddr` precisely for this reason — it's the address you allocated from and must return.
**How to apply:** Every `Hal::dma_dealloc` implementation should use `vaddr.as_ptr()`. The `paddr` parameter is for IOMMU programming, not for deallocation.

## Use Hardware CSR `time::read()` for Elapsed Time — Not Software Ticks

When measuring elapsed time inside the kernel (e.g., for hang detection), use `riscv::register::time::read()` (CSR 0xC01), not `crate::task::system_ticks()`.

**Why:** `system_ticks()` reads a software counter incremented by `task::tick()`. As of 2026-05-29, `tick()` has **zero callers** — the timer ISR in `hal/arch/riscv/src/rv64/trap.rs` is a stub. A check against `system_ticks()` always sees 0 elapsed and can never fire. The hardware TIME CSR is monotonic and immediately available (10 MHz on QEMU virt → 10 M ticks ≈ 1 second).
**How to apply:** `riscv::register::time::read()` is available in `riscv = "0.16.0"` (kernel/Cargo.toml). Use it for any "detect hung device" pattern. Document the assumed tick rate (10 MHz QEMU virt) near the threshold constant.

## Block Device `flush()` — Must Check Device Presence

A `ViBlockDevice::flush()` implementation must return `Err(NotFound)` if the device was never probed:
```rust
fn flush(&self) -> ViResult<()> {
    if BLOCK_DEVICE.lock().is_some() { Ok(()) } else { Err(ViError::NotFound) }
}
```

**Why:** If `flush()` returns `Ok(())` unconditionally, a caller that issues `write_sector() + flush()` believes writes are durable even when no device exists. This silent lie is worse than an error. Same pattern applies to GPU flush, NIC packet-send, etc.
**How to apply:** Any "synchronous" device flush can return Ok if the device is present (VirtIO write_blocks already waits for completion). Device-absent case must be Err.

## Kernel Integration Tests — Defer Until Phase 11

The kernel (`ViCell-kernel`) is a **binary crate** with no `[lib]` section. Rust integration tests require a library crate (or `#![feature(custom_test_frameworks)]` + a `no_std`-compatible harness). Do NOT create `tests/` directories or `.rs` files under `tests/integration/` until Phase 11 sets up the test harness.

**Why:** A test file in `tests/` that isn't a workspace member, or that tries to import a binary crate, will not compile. The reviewer flagged `tests/integration/virtio_block.rs` as dead orphan code (deleted). Misleading "test exists" claims in commit messages are worse than no test.
**How to apply:** Kernel smoke tests live inline in the kernel (gated on a feature flag) until Phase 11 wires up the harness. Phase 11 should declare `[lib]` in `kernel/Cargo.toml` or use a `custom_test_frameworks` harness.

## `// SAFETY:` on extern "Rust" FFI calls in trap.rs

Every `unsafe { vi_handle_*() }` call in `hal/arch/riscv/src/rv64/trap.rs` needs a `// SAFETY:` comment per Law 4 (Unsafe Management in CLAUDE.md).

**Why:** Code review flagged missing SAFETY comments as a low-priority finding. CLAUDE.md Law 4 is absolute: no unsafe block without SAFETY justification. These are `extern "Rust"` symbols linked from the kernel — the invariant to document is "function is defined in kernel and linked at link time; argument is a valid PLIC claim value".
**How to apply:** When adding new trap dispatch entries (e.g., for GPU or NIC IRQs), always add the SAFETY comment inline before the unsafe block.
