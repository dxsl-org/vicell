//! AArch64 exception vectors and trap handlers.
use core::arch::global_asm;

/// Saved register state on entry to a trap handler.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct TrapFrame {
    pub regs: [u64; 31],
    pub elr_el1:  u64,
    pub spsr_el1: u64,
    pub far_el1:  u64,
    pub esr_el1:  u64,
}

/// Install the vector table.
pub fn init() {
    extern "C" { static __vectors: u8; }
    let vbar = unsafe { &__vectors as *const u8 as u64 };
    // SAFETY: VBAR_EL1 is EL1-private; address is 2048-byte aligned (enforced by .balign).
    unsafe {
        core::arch::asm!("msr vbar_el1, {}", in(reg) vbar, options(nomem, nostack));
    }
}

/// Synchronous trap dispatcher.
#[no_mangle]
pub extern "C" fn vi_aarch64_trap_handler(frame: &mut TrapFrame) {
    let esr = frame.esr_el1;
    let ec  = (esr >> 26) & 0x3F;
    match ec {
        0x15 => {
            frame.regs[0] = usize::MAX as u64;
        }
        _ => {
            panic!("[aarch64] trap ec=0x{:X} esr=0x{:X} elr=0x{:X}", ec, esr, frame.elr_el1);
        }
    }
}

/// IRQ handler.
#[no_mangle]
pub extern "C" fn vi_aarch64_irq_handler(_frame: &mut TrapFrame) {
    let irq = super::gic::claim();
    if irq != 0x3FF { super::gic::complete(irq); }
}

global_asm!(
r#"
    .macro SAVE_REGS
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
    .endm
    .macro RESTORE_REGS
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
    .endm
    .section .text.vectors
    .global __vectors
    .balign 2048
__vectors:
    .balign 0x80; SAVE_REGS; mov x0, sp; bl vi_aarch64_trap_handler; RESTORE_REGS; eret
    .balign 0x80; SAVE_REGS; mov x0, sp; bl vi_aarch64_irq_handler;  RESTORE_REGS; eret
    .balign 0x80; SAVE_REGS; mov x0, sp; bl vi_aarch64_trap_handler; RESTORE_REGS; eret
    .balign 0x80; SAVE_REGS; mov x0, sp; bl vi_aarch64_trap_handler; RESTORE_REGS; eret
    .balign 0x80; SAVE_REGS; mov x0, sp; bl vi_aarch64_trap_handler; RESTORE_REGS; eret
    .balign 0x80; SAVE_REGS; mov x0, sp; bl vi_aarch64_irq_handler;  RESTORE_REGS; eret
    .balign 0x80; SAVE_REGS; mov x0, sp; bl vi_aarch64_trap_handler; RESTORE_REGS; eret
    .balign 0x80; SAVE_REGS; mov x0, sp; bl vi_aarch64_trap_handler; RESTORE_REGS; eret
    .balign 0x80; SAVE_REGS; mov x0, sp; bl vi_aarch64_trap_handler; RESTORE_REGS; eret
    .balign 0x80; SAVE_REGS; mov x0, sp; bl vi_aarch64_irq_handler;  RESTORE_REGS; eret
    .balign 0x80; SAVE_REGS; mov x0, sp; bl vi_aarch64_trap_handler; RESTORE_REGS; eret
    .balign 0x80; SAVE_REGS; mov x0, sp; bl vi_aarch64_trap_handler; RESTORE_REGS; eret
    .balign 0x80; b .
    .balign 0x80; b .
    .balign 0x80; b .
    .balign 0x80; b .
"#
);
