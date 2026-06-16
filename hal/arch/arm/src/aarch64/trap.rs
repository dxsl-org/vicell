//! AArch64 exception vectors and trap handlers.
//!
//! At EL1 (default): installs `__vectors` into VBAR_EL1.
//! At EL2 (virtualization=on): installs `__vectors_el2` into VBAR_EL2.
//! The runtime dispatch is driven by `EL2_ACTIVE` (set in el2.rs at boot).

use core::arch::global_asm;

/// Saved register state on entry to a trap handler.
///
/// Field names use `_el1` suffixes matching the EL1 register names; at EL2
/// the assembly saves `elr_el2`/`spsr_el2`/`far_el2`/`esr_el2` into the same
/// offsets — the struct is a plain bag of u64 and the names are irrelevant at
/// the Rust level.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct TrapFrame {
    pub regs: [u64; 31],
    pub elr_el1:  u64,  // offset 248 — holds ELR_EL2 at runtime when EL2 active
    pub spsr_el1: u64,  // offset 256
    pub far_el1:  u64,  // offset 264
    pub esr_el1:  u64,  // offset 272
}

/// Mirror of `hal_riscv::rv64::trap::ViTrapFrame` — same `#[repr(C)]` layout.
/// Needed because hal-arm does not depend on hal-riscv; both are `#[repr(C)]`
/// so the binary call to `ViCell_syscall_dispatch` is well-defined by layout.
#[derive(Default, Clone, Copy)]
#[repr(C)]
struct ViTrapFrameBridge {
    pub regs:    [usize; 32],
    pub sstatus: usize,
    pub sepc:    usize,
    pub stval:   usize,
    pub scause:  usize,
}

/// Bridge ARM64 SVC registers into the kernel's generic syscall dispatcher.
fn svc_dispatch(frame: &mut TrapFrame) {
    extern "Rust" {
        fn ViCell_syscall_dispatch(frame: &mut ViTrapFrameBridge);
    }
    let mut vtf = ViTrapFrameBridge::default();
    vtf.regs[17] = frame.regs[0] as usize; // syscall number (x0)
    vtf.regs[10] = frame.regs[1] as usize; // a0 (x1)
    vtf.regs[11] = frame.regs[2] as usize; // a1 (x2)
    vtf.regs[12] = frame.regs[3] as usize; // a2 (x3)
    vtf.regs[13] = frame.regs[4] as usize; // a3 (x4)
    // elr_el1 holds ELR_EL1 at EL1, or ELR_EL2 at EL2 — both are the
    // return address past the SVC instruction that the kernel needs.
    vtf.sepc     = frame.elr_el1 as usize;
    // SAFETY: ViTrapFrameBridge is layout-identical to hal_riscv::ViTrapFrame
    // (both #[repr(C)], same fields and order). The kernel side is #[no_mangle]
    // extern "Rust" and will be resolved to the same symbol at link time.
    unsafe { ViCell_syscall_dispatch(&mut vtf); }
    frame.regs[0] = vtf.regs[10] as u64; // return value → x0
}

/// Install the exception vector table.
///
/// At EL1: writes `__vectors` into VBAR_EL1.
/// At EL2: writes `__vectors_el2` into VBAR_EL2.
pub fn init() {
    extern "C" {
        static __vectors: u8;
        static __vectors_el2: u8;
    }
    if super::el2::is_el2() {
        let vbar = unsafe { &__vectors_el2 as *const u8 as u64 };
        // SAFETY: VBAR_EL2 is EL2-private; address is 2048-byte aligned
        // (enforced by `.balign 2048` in el2.rs global_asm).
        unsafe { core::arch::asm!("msr vbar_el2, {}", in(reg) vbar, options(nomem, nostack)); }
    } else {
        let vbar = unsafe { &__vectors as *const u8 as u64 };
        // SAFETY: VBAR_EL1 is EL1-private; address is 2048-byte aligned.
        unsafe { core::arch::asm!("msr vbar_el1, {}", in(reg) vbar, options(nomem, nostack)); }
    }
}

/// Synchronous trap dispatcher — called from both EL1 and EL2 trampolines.
#[no_mangle]
pub extern "C" fn vi_aarch64_trap_handler(frame: &mut TrapFrame) {
    let esr = frame.esr_el1; // field holds ESR_EL2 at EL2; naming is irrelevant here
    let ec  = (esr >> 26) & 0x3F;
    match ec {
        // EC 0x15 = SVC instruction from AArch64.
        // ViCell ARM64 syscall ABI: x0=syscall_nr, x1=a0, x2=a1, x3=a2, x4=a3.
        0x15 => {
            svc_dispatch(frame);
        }
        // EC 0x20 = Instruction Abort from lower EL (EL0 cell).
        // EC 0x24 = Data Abort from lower EL (EL0 cell).
        // With HCR_EL2.TGE=1 all EL0 exceptions trap to EL2 — these ECs only
        // arrive from EL0 in our setup (there is no EL1 guest).
        // Forward to the kernel fault handler: kills the cell, lets the OS continue.
        0x20 | 0x24 => {
            extern "Rust" {
                fn vi_terminate_on_fault(scause: usize, sepc: usize, stval: usize);
            }
            // SAFETY: vi_terminate_on_fault is #[no_mangle] in kernel::task.
            // It force-unlocks all kernel locks, sends NotifyOnExit, and calls
            // yield_cpu() which switches away from this (now dead) cell.
            unsafe {
                vi_terminate_on_fault(
                    esr as usize,           // fault class (ESR_EL2)
                    frame.elr_el1 as usize, // faulting instruction PC (ELR_EL2)
                    frame.far_el1 as usize, // faulting address (FAR_EL2)
                );
            }
        }
        _ => {
            panic!("[aarch64] trap ec=0x{:X} esr=0x{:X} elr=0x{:X} far=0x{:X}",
                ec, esr, frame.elr_el1, frame.far_el1);
        }
    }
}

/// GIC ID for the PL061 GPIO controller on QEMU ARM virt (SPI 7 = GIC ID 39).
const GPIO_GIC_ID: u32 = 39;

/// IRQ handler — dispatches timer, GPIO, and VirtIO MMIO interrupts.
///
/// Timer PPIs: EL1 = GIC ID 30 (CNTP), EL2 = GIC ID 26 (CNTHP).
/// GPIO PL061: GIC ID 39 (SPI 7) → forwards to MMIO-owner cell via IPC.
/// VirtIO MMIO: QEMU virt SPI 16..47 = GIC IDs 48..79 (32 slots).
/// `vi_handle_virtio_irq` expects SPI numbers (as stored in `VirtioEntry.irq`),
/// so GIC ID is converted: SPI_nr = GIC_ID − 32.
#[no_mangle]
pub extern "C" fn vi_aarch64_irq_handler(_frame: &mut TrapFrame) {
    extern "Rust" {
        fn vi_timer_tick();
        fn vi_handle_virtio_irq(irq: u32);
        fn vi_gpio_notify_irq();
    }
    let irq = super::gic::claim();
    let timer_irq = if super::el2::is_el2() { 26 } else { 30 };
    if irq == timer_irq {
        // Rearm the hardware countdown first.
        super::timer::reset();
        // Send EOI (priority drop) BEFORE calling vi_timer_tick().
        // vi_timer_tick() calls yield_cpu() which context-switches away.
        // GICv2: until GICC_EOIR is written, the IRQ stays "active" and the
        // GIC priority preemption logic blocks all same/lower priority IRQs
        // (all VirtIO + timer share priority 0xA0).  If we EOI after yield,
        // the interrupted task must be rescheduled before the next timer tick
        // can fire — but that task may be stuck in Sending{} waiting for an
        // IPC reply whose sender (net) can only wake via a timer timeout.
        // EOI first → priority drop → new timer ticks can fire on any task.
        super::gic::complete(irq);
        // SAFETY: vi_timer_tick is #[no_mangle] in kernel/src/task.rs.
        unsafe { vi_timer_tick(); }
        return; // skip the final complete() below — EOI already sent
    } else if irq == GPIO_GIC_ID {
        // GPIO PL061 edge interrupt: EOI first (priority drop), then notify cell.
        // EOI before notify so the GIC can deliver the next GPIO edge immediately
        // after the cell re-enables the interrupt (GPIOIE write from userspace).
        super::gic::complete(irq);
        // SAFETY: vi_gpio_notify_irq is #[no_mangle] in kernel/src/task/drivers/gpio_irq.rs.
        unsafe { vi_gpio_notify_irq(); }
        return;
    } else if irq >= 32 && irq != 0x3FF {
        // SPI range (GIC ID ≥ 32): dispatch to the VirtIO IRQ handler.
        // Convert GIC ID → SPI number so the comparison against VirtioEntry.irq works.
        // SAFETY: vi_handle_virtio_irq is #[no_mangle] in kernel/src/task/drivers.
        unsafe { vi_handle_virtio_irq(irq - 32); }
    }
    if irq != 0x3FF { super::gic::complete(irq); }
}

/// Noop: ARM64 uses SP_EL0 via context switch, not an sscratch-style CSR.
pub fn set_kernel_stack(_top: usize) {}

/// Unmask IRQs by clearing DAIF.I.
pub fn enable_interrupts() {
    // SAFETY: msr daifclr from EL1/EL2 is always permitted.
    unsafe { core::arch::asm!("msr daifclr, #2", options(nomem, nostack)); }
}

/// ARM64 has no GP/TP registers — return zeroes so kernel spawn paths compile.
pub fn get_gp_tp() -> (usize, usize) { (0, 0) }

global_asm!(r#"
    .section .text
    .global thread_trampoline
    .balign 4
thread_trampoline:
    msr daifclr, #2          // enable IRQ (I bit cleared)
    mov x0, x19              // arg  (s0-equiv stored in x19 by spawn setup)
    br  x20                  // entry (s1-equiv stored in x20 by spawn setup)
"#);

global_asm!(r#"
    // __trap_exit — restore ViTrapFrame from the kernel stack and eret to EL0.
    //
    // Called when a spawned task runs for the first time (context.x30 = __trap_exit).
    // On entry: sp → arch::ViTrapFrame (288 bytes, layout: regs[32], sstatus, sepc, stval, scause).
    //
    // Offsets: regs[N] = N*8; sstatus = 256; sepc = 264.
    //
    // Runtime dispatch: reads EL2_ACTIVE (1 byte, AtomicBool) at boot-time-set address.
    // EL1 path: msr elr_el1, spsr_el1.
    // EL2 path: msr elr_el2, spsr_el2.
    .section .text
    .global __trap_exit
    .balign 4
__trap_exit:
    // Runtime EL dispatch via EL2_ACTIVE flag.
    // SAFETY: EL2_ACTIVE is an AtomicBool (1 byte); ldrb loads it atomically
    // for reads (store-release in el2_mark_active provides the ordering guarantee).
    adrp  x9, EL2_ACTIVE
    add   x9, x9, :lo12:EL2_ACTIVE
    ldrb  w9, [x9]
    cbnz  w9, 1f

    // ── EL1 path ─────────────────────────────────────────────────────────────
    ldr  x9,  [sp, #264]     // sepc → ELR_EL1 (user entry point)
    msr  elr_el1, x9
    mov  x9,  #0
    msr  spsr_el1, x9         // EL0t, no interrupt masking
    ldr  x9,  [sp, #16]      // regs[2] = user sp
    msr  sp_el0, x9
    ldp  x0,  x1,  [sp, #0]
    ldp  x2,  x3,  [sp, #16]
    ldp  x4,  x5,  [sp, #32]
    ldp  x6,  x7,  [sp, #48]
    ldp  x8,  x9,  [sp, #64]
    ldp  x10, x11, [sp, #80]
    ldp  x12, x13, [sp, #96]
    ldp  x14, x15, [sp, #112]
    ldp  x16, x17, [sp, #128]
    ldp  x18, x19, [sp, #144]
    ldp  x20, x21, [sp, #160]
    ldp  x22, x23, [sp, #176]
    ldp  x24, x25, [sp, #192]
    ldp  x26, x27, [sp, #208]
    ldp  x28, x29, [sp, #224]
    ldr  x30,       [sp, #240]
    add  sp, sp, #288
    eret

    // ── EL2 path ─────────────────────────────────────────────────────────────
1:
    ldr  x9,  [sp, #264]     // sepc → ELR_EL2 (user entry point)
    msr  elr_el2, x9
    mov  x9,  #0
    msr  spsr_el2, x9         // EL0t — Cells stay at EL0
    ldr  x9,  [sp, #16]      // regs[2] = user sp
    msr  sp_el0, x9
    ldp  x0,  x1,  [sp, #0]
    ldp  x2,  x3,  [sp, #16]
    ldp  x4,  x5,  [sp, #32]
    ldp  x6,  x7,  [sp, #48]
    ldp  x8,  x9,  [sp, #64]
    ldp  x10, x11, [sp, #80]
    ldp  x12, x13, [sp, #96]
    ldp  x14, x15, [sp, #112]
    ldp  x16, x17, [sp, #128]
    ldp  x18, x19, [sp, #144]
    ldp  x20, x21, [sp, #160]
    ldp  x22, x23, [sp, #176]
    ldp  x24, x25, [sp, #192]
    ldp  x26, x27, [sp, #208]
    ldp  x28, x29, [sp, #224]
    ldr  x30,       [sp, #240]
    add  sp, sp, #288
    eret
"#);

global_asm!(
r#"
    // AArch64 EL1 vector table — ARM spec requires each entry at VBAR + N*0x80.
    // SAVE_REGS + branch + RESTORE_REGS + eret = ~188 bytes which overflows the
    // 128-byte (0x80) slot.  Use a single `b` per slot branching to out-of-line
    // trampolines that have no size constraint.
    .section .text.vectors
    .global __vectors
    .balign 2048
__vectors:
    // ── Current EL, SP_EL0 ──────────────────────────────────────────────────
    .balign 0x80; b vt_sync_sp0
    .balign 0x80; b vt_irq_sp0
    .balign 0x80; b vt_sync_sp0        // FIQ → treat as sync
    .balign 0x80; b vt_sync_sp0        // SError → treat as sync
    // ── Current EL, SP_ELx ──────────────────────────────────────────────────
    .balign 0x80; b vt_sync_spx
    .balign 0x80; b vt_irq_spx
    .balign 0x80; b vt_sync_spx
    .balign 0x80; b vt_sync_spx
    // ── Lower EL (AArch64) ───────────────────────────────────────────────────
    .balign 0x80; b vt_sync_el0
    .balign 0x80; b vt_irq_el0
    .balign 0x80; b vt_sync_el0
    .balign 0x80; b vt_sync_el0
    // ── Lower EL (AArch32) ── not supported ─────────────────────────────────
    .balign 0x80; b .
    .balign 0x80; b .
    .balign 0x80; b .
    .balign 0x80; b .

    // ── Out-of-line trampolines ──────────────────────────────────────────────
    // TrapFrame layout (35 * 8 = 280 bytes):
    //   x0..x30  at offsets 0..240 (each 8 bytes)
    //   elr_el1  at 248
    //   spsr_el1 at 256
    //   far_el1  at 264
    //   esr_el1  at 272
    .section .text
    .balign 4
vt_sync_sp0:
vt_sync_spx:
vt_sync_el0:
    sub  sp, sp, #(35 * 8)
    stp  x0,  x1,  [sp, #0]
    stp  x2,  x3,  [sp, #16]
    stp  x4,  x5,  [sp, #32]
    stp  x6,  x7,  [sp, #48]
    stp  x8,  x9,  [sp, #64]
    stp  x10, x11, [sp, #80]
    stp  x12, x13, [sp, #96]
    stp  x14, x15, [sp, #112]
    stp  x16, x17, [sp, #128]
    stp  x18, x19, [sp, #144]
    stp  x20, x21, [sp, #160]
    stp  x22, x23, [sp, #176]
    stp  x24, x25, [sp, #192]
    stp  x26, x27, [sp, #208]
    stp  x28, x29, [sp, #224]
    str  x30,       [sp, #240]
    mrs  x9,  elr_el1
    mrs  x10, spsr_el1
    mrs  x11, far_el1
    mrs  x12, esr_el1
    stp  x9,  x10, [sp, #248]
    stp  x11, x12, [sp, #264]
    mov  x0,  sp
    bl   vi_aarch64_trap_handler
    ldp  x9,  x10, [sp, #248]
    msr  elr_el1,  x9
    msr  spsr_el1, x10
    ldp  x0,  x1,  [sp, #0]
    ldp  x2,  x3,  [sp, #16]
    ldp  x4,  x5,  [sp, #32]
    ldp  x6,  x7,  [sp, #48]
    ldp  x8,  x9,  [sp, #64]
    ldp  x10, x11, [sp, #80]
    ldp  x12, x13, [sp, #96]
    ldp  x14, x15, [sp, #112]
    ldp  x16, x17, [sp, #128]
    ldp  x18, x19, [sp, #144]
    ldp  x20, x21, [sp, #160]
    ldp  x22, x23, [sp, #176]
    ldp  x24, x25, [sp, #192]
    ldp  x26, x27, [sp, #208]
    ldp  x28, x29, [sp, #224]
    ldr  x30,       [sp, #240]
    add  sp, sp, #(35 * 8)
    eret

    .balign 4
vt_irq_sp0:
vt_irq_spx:
vt_irq_el0:
    sub  sp, sp, #(35 * 8)
    stp  x0,  x1,  [sp, #0]
    stp  x2,  x3,  [sp, #16]
    stp  x4,  x5,  [sp, #32]
    stp  x6,  x7,  [sp, #48]
    stp  x8,  x9,  [sp, #64]
    stp  x10, x11, [sp, #80]
    stp  x12, x13, [sp, #96]
    stp  x14, x15, [sp, #112]
    stp  x16, x17, [sp, #128]
    stp  x18, x19, [sp, #144]
    stp  x20, x21, [sp, #160]
    stp  x22, x23, [sp, #176]
    stp  x24, x25, [sp, #192]
    stp  x26, x27, [sp, #208]
    stp  x28, x29, [sp, #224]
    str  x30,       [sp, #240]
    mrs  x9,  elr_el1
    mrs  x10, spsr_el1
    mrs  x11, far_el1
    mrs  x12, esr_el1
    stp  x9,  x10, [sp, #248]
    stp  x11, x12, [sp, #264]
    mov  x0,  sp
    bl   vi_aarch64_irq_handler
    ldp  x9,  x10, [sp, #248]
    msr  elr_el1,  x9
    msr  spsr_el1, x10
    ldp  x0,  x1,  [sp, #0]
    ldp  x2,  x3,  [sp, #16]
    ldp  x4,  x5,  [sp, #32]
    ldp  x6,  x7,  [sp, #48]
    ldp  x8,  x9,  [sp, #64]
    ldp  x10, x11, [sp, #80]
    ldp  x12, x13, [sp, #96]
    ldp  x14, x15, [sp, #112]
    ldp  x16, x17, [sp, #128]
    ldp  x18, x19, [sp, #144]
    ldp  x20, x21, [sp, #160]
    ldp  x22, x23, [sp, #176]
    ldp  x24, x25, [sp, #192]
    ldp  x26, x27, [sp, #208]
    ldp  x28, x29, [sp, #224]
    ldr  x30,       [sp, #240]
    add  sp, sp, #(35 * 8)
    eret
"#
);
