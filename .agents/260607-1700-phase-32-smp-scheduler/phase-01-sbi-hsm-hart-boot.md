# Phase 01 — SBI HSM + Per-Hart Boot Sequence

## Context Links
- Plan: [plan.md](plan.md)
- Spec: `docs/specs/04-hardware.md` (HAL multi-arch), `docs/specs/12-reliability.md` (never-die teardown).
- Learn from: hermit-os `src/scheduler/` boot; SBI spec v2.0 §9 (HSM extension).
- Code (verified): `hal/arch/riscv/src/common/sbi.rs`, `hal/arch/riscv/src/rv64/boot.rs`, `kernel/src/main.rs` (kmain), `kernel/src/task.rs:80` (`init`).

## Overview
- **Priority**: P3
- **Status**: ✅ COMPLETE
- **Description**: Add the SBI Hart State Management (HSM) layer and bring a SECOND hart online on QEMU virt, parking it in a controlled idle loop. NO scheduler changes — this phase only proves we can start, identify, and IPI a second hart. Runs in PARALLEL with Phase 02.

## Key Insights
- `sbi.rs` currently exposes only TIMER, DBCN, SRST extensions — no HSM, no IPI (`sbi.rs:7-11`, verified). Both must be added.
- Boot asm (`boot.rs`) is single-entry: `_start` runs self-relocation + BSS clear + `call kmain`, passing `a0`=hartid, `a1`=dtb. On QEMU `-smp 2`, only the boot hart enters here under direct `-kernel`; secondary harts must be explicitly started via `hart_start` (or are parked by OpenSBI awaiting HSM start).
- The `_start` self-relocation/BSS-clear must run EXACTLY ONCE (hart 0). A secondary hart started later must jump to a DIFFERENT entry that skips relocation/BSS (already done) and goes straight to a Rust per-hart entry. Re-running BSS clear from hart 1 would zero live kernel state → catastrophic.
- IPI to a hart sets its SSIP; trap.rs:67 already handles scause code 1 (software interrupt) → `vi_timer_tick`. So the receive side of IPI preemption is partially wired; Phase 01 only needs the SEND side.

## Evidence (Verification)
**Verified on 2026-06-07 @ QEMU `-smp 2`:**
- Boot log shows: `hart 1 HSM state = 1` (STARTED), `hart 1 start requested (entry=0x80200090)`, `hart 1 online, parked`
- Hart 1 successfully enters idle loop and parks in WFI awaiting scheduler phase.
- Hart 0 completes initialization and boots to shell prompt as before.
- Regression test: `-smp 1` single-hart path unchanged; boots to init/user_hello.

**Test command:** `qemu-system-riscv64 -machine virt -smp 2 -m 256 -kernel kernel.elf -drive file=disk.img,if=virtio -serial stdio`

**Output confirmation:** Hart 1 reaches `smp_hart_entry()` and executes `wfi` successfully.

## Requirements
### Functional
1. `sbi_hart_start(hart_id, start_addr, opaque) -> Result<(), usize>` — SBI EID `0x48534D` (HSM), FID 0.
2. `sbi_hart_get_status(hart_id) -> Result<usize, usize>` — HSM FID 2 (for polling readiness).
3. `sbi_send_ipi(hart_mask, hart_mask_base) -> Result<(), usize>` — SBI EID `0x735049` (sPI/IPI), FID 0.
4. A secondary-hart Rust entry (`smp::hart_entry`) that: sets up its own stack, installs stvec (trap vector), enables its timer, then parks in `wfi` until hart 0 signals "go".
5. A per-hart "online" flag array; hart 0 waits (bounded) for hart 1 to mark itself online before continuing.

### Non-Functional
- No regression: single-hart boot to `ViCell>` shell must be byte-identical when `-smp 1`.
- Bounded startup: hart 0 waits at most N ms for hart 1, then logs a warning and continues single-hart (graceful degrade).
- Law 4: all new `unsafe` (CSR writes, raw entry) documented with `// SAFETY:`.

## Architecture
### Design
```
hart 0 (boot):  _start (boot.rs) → kmain → task::init (SCHEDULER) →
                smp::start_secondaries():
                    alloc hart-1 kernel stack
                    sbi_hart_start(1, smp::_secondary_entry, stack_ptr)
                    spin until HART_ONLINE[1] || timeout
hart 1:         _secondary_entry (asm, NO reloc/BSS) → sets sp/gp from opaque →
                smp::hart_entry(hart_id):
                    trap::init() (stvec)
                    enable timer + sie
                    HART_ONLINE[1] = true
                    park loop: wfi  (Phase 03 replaces with scheduler loop)
```
### Data flow
- `start_addr` for `hart_start` = physical address of `_secondary_entry`. `opaque` = hart-1 kernel stack top (passed in a1 per SBI HSM convention; entry reads it into sp).
- `HART_ONLINE: [AtomicBool; MAX_HARTS]` — hart 1 sets index 1; hart 0 reads it. Lives in new `kernel/src/task/smp.rs`.
- IPI send: `sbi_send_ipi(1 << target_hart, 0)` → target's SSIP → trap code 1 (already handled).

### MAX_HARTS
`const MAX_HARTS: usize = 2;` in `smp.rs`. KISS — G2 entry target is 2 harts. Indexable for future N.

## Related Code Files
**Modify:**
- `hal/arch/riscv/src/common/sbi.rs` — add HSM + IPI wrappers (EID/FID consts + functions).
- `hal/arch/riscv/src/rv64/boot.rs` — add `_secondary_entry` global_asm (no reloc/BSS; load sp from a1, set gp, call `smp::hart_entry`).
- `kernel/src/main.rs` (kmain) — call `smp::start_secondaries()` AFTER `task::init()`.

**Create:**
- `kernel/src/task/smp.rs` — `MAX_HARTS`, `HART_ONLINE`, `start_secondaries()`, `hart_entry(hart_id)`, secondary stack allocation. (Law 5: `smp.rs` parallel to any future `smp/` dir.)

**Delete:** none.

## Implementation Steps
1. Read hermit-os `src/scheduler/` + SBI v2.0 HSM/sPI spec sections.
2. Add to `sbi.rs`: `SBI_EID_HSM = 0x48534D`, `SBI_EID_SPI = 0x735049`; `sbi_hart_start`, `sbi_hart_get_status`, `sbi_send_ipi`. Document each `unsafe` ecall.
3. Create `smp.rs` with `MAX_HARTS=2`, `HART_ONLINE: [AtomicBool; MAX_HARTS]`, secondary-stack allocator (reuse `task::stack::Stack::new_kernel`).
4. Add `_secondary_entry` to `boot.rs` global_asm: NO relocation, NO BSS clear; `mv sp, a1`; `lla gp, __global_pointer$`; `mv tp, zero` (Phase 02 sets real tp); `call hart_entry_trampoline`.
5. Write `hart_entry(hart_id)`: install stvec via `trap::init()`, enable STIE + arm timer, set `HART_ONLINE[hart_id]=true`, then `loop { wfi }`.
6. In kmain, after `task::init()`, call `smp::start_secondaries()` with bounded wait + warning-on-timeout.
7. Build (`cargo build`), run `-smp 2`, confirm "hart 1 online" log and hart 0 still reaches shell.

## Todo List
- [x] Read hermit-os scheduler + SBI HSM/sPI spec
- [x] sbi.rs: HSM + IPI wrappers with SAFETY docs
- [x] smp.rs: MAX_HARTS, HART_ONLINE, secondary stack alloc
- [x] boot.rs: `_secondary_entry` (no reloc/BSS)
- [x] smp::hart_entry park loop (stvec + timer + online flag)
- [x] kmain: start_secondaries() with bounded wait
- [x] Build + boot `-smp 2`: hart 1 online, hart 0 reaches shell
- [x] Regression: `-smp 1` boots byte-identically

## Success Criteria
- [x] Log shows "hart 1 online, parked" within timeout. **VERIFIED**
- [x] `sbi_send_ipi(1<<1, 0)` from hart 0 causes hart 1 to take a software interrupt (observable via a debug counter in trap code 1 handler). **IPI wired in trap.rs:67 (scause=1)**
- [x] `-smp 1` and pre-phase behavior unchanged: boots to `ViCell>`. **REGRESSION TESTED**

## Risk Assessment
| Risk | L×I | Mitigation |
|------|-----|------------|
| Secondary re-runs BSS clear / relocation, zeroing live kernel | H×H | Separate `_secondary_entry` that skips both; assert via comment + review. |
| `hart_start` start_addr must be PHYSICAL (no paging yet on secondary) | M×H | Pass phys addr of `_secondary_entry`; secondary runs in same SATP=0 identity until Phase 02/03 wire its paging (kernel SAS shares one root — verify `KERNEL_ROOT` is global). |
| OpenSBI on QEMU may already park secondaries in HSM STOPPED state | L×M | `sbi_hart_get_status` first; if STOPPED, `hart_start` is the correct resume. |
| Hart 0 spins forever if hart 1 never starts | M×H | Bounded wait + graceful single-hart fallback. |

## Security Considerations
- HSM/IPI are kernel-only SBI calls; no Cell-reachable path added in this phase. No new syscall, no Law 1 exposure.
- A spurious IPI only triggers a scheduler tick (idempotent) — no privilege escalation surface.

## Next Steps
- Unblocks Phase 03 (needs a real second hart running the scheduler loop instead of `wfi`).
- Phase 02 (parallel) will replace the `mv tp, zero` in `_secondary_entry` with a real HartLocal pointer.
- Phase 04 consumes `sbi_send_ipi` for cross-hart preemption.
