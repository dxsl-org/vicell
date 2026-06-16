//! AArch32 (ARMv7-A) boot entry — _start in SVC mode.
//!
//! QEMU ARM virt `-kernel` loads the ELF at 0x40080000 and enters `_start`
//! in SVC mode (CPSR bits[4:0] = 0x13), MMU OFF, r0=0, r1=machine-type, r2=DTB.
//!
//! Entry sequence:
//!   1. Disable IRQ + FIQ (cpsid if)
//!   2. Save DTB pointer (r2 → r8)
//!   3. Load SP from .Lstack_ptr literal pool
//!   4. Zero BSS section
//!   5. Call kmain_arm32(hartid=0, dtb=saved-r2)

use core::arch::global_asm;

global_asm!(
    ".section .text.boot, \"ax\"",
    ".global _start",
    "_start:",
    // Disable IRQ and FIQ (CPSR.I=1, CPSR.F=1).
    "cpsid if",
    // Save DTB pointer (r2 may be clobbered by BSS clear).
    "mov r8, r2",
    // Load stack pointer from literal pool.
    "ldr sp, .Lstack_ptr",
    // Zero BSS.
    "mov r6, #0",
    "ldr r4, .Lbss_start",
    "ldr r5, .Lbss_end",
    ".Lbss_loop:",
    "cmp r4, r5",
    "bge .Lbss_done",
    "str r6, [r4], #4",
    "b .Lbss_loop",
    ".Lbss_done:",
    // Call kmain_arm32(hartid=0, dtb=r8).
    "mov r0, #0",
    "mov r1, r8",
    "bl {entry}",
    ".Lhalt:",
    "wfi",
    "b .Lhalt",

    // Literal pool — must be reachable from the PC-relative LDR above.
    ".align 2",
    ".Lstack_ptr:  .word {stack_top}",
    ".Lbss_start:  .word __bss_start",
    ".Lbss_end:    .word __bss_end",

    // ── Thread trampoline stub (required symbol; never called in nano) ─────────
    ".global thread_trampoline",
    "thread_trampoline:",
    ".Ltrap_halt:",
    "wfi",
    "b .Ltrap_halt",

    stack_top = sym __stack_top,
    entry     = sym kmain_arm32,
);

/// Bridge from 32-bit ARM `_start` to the architecture-neutral `kmain`.
#[no_mangle]
pub extern "C" fn kmain_arm32(hartid: usize, dtb: usize) -> ! {
    extern "C" { fn kmain(hartid: usize, dtb: usize) -> !; }
    // SAFETY: kmain is the kernel entry; hartid/dtb forwarded from QEMU entry state.
    unsafe { kmain(hartid, dtb) }
}

// ── Linker-script symbols ────────────────────────────────────────────────────
extern "C" {
    pub static __stack_top: u8;
    pub static mut __bss_start: u8;
    pub static __bss_end: u8;
}
