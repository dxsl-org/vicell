//! AArch64 boot entry point.
//!
//! Handles the EL2→EL1 transition (if entered at EL2), clears BSS,
//! sets up an early stack, and branches to `kmain`.

use core::arch::global_asm;

global_asm!(
    r#"
    .section .text.boot
    .global _start
    .balign 4
_start:
    // Disable all interrupts (DAIF = 0b1111).
    msr daifset, #0xf

    // Stash DTB pointer (x0 on QEMU virt) in x19 (callee-saved) before it
    // is clobbered by the BSS-clear loop and stack setup.
    mov  x19, x0  // DTB physical address

    // Determine current exception level.
    mrs x0, CurrentEL
    lsr x0, x0, #2          // CurrentEL[3:2]
    cmp x0, #2
    b.eq .el2_to_el1
    b .el1_entry             // Already in EL1

.el2_to_el1:
    // Configure HCR_EL2: set RW=1 (EL1 runs AArch64), not hosting a hypervisor.
    mov x0, #(1 << 31)      // RW bit
    msr hcr_el2, x0

    // Set SPSR_EL2 so eret enters EL1h (EL1 with SP_EL1).
    mov x0, #0x3C5          // EL1h, DAIF masked
    msr spsr_el2, x0

    // Point ELR_EL2 to el1_entry so eret transfers there.
    adr x0, .el1_entry
    msr elr_el2, x0
    eret

.el1_entry:
    // Set up initial stack at __stack_top (defined in linker script).
    adrp x0, __stack_top
    add  x0, x0, :lo12:__stack_top
    mov  sp, x0

    // Clear BSS section.
    adrp x0, __bss_start
    add  x0, x0, :lo12:__bss_start
    adrp x1, __bss_end
    add  x1, x1, :lo12:__bss_end
1:
    cmp  x0, x1
    b.hs 2f
    str  xzr, [x0], #8
    b    1b
2:
    // Jump to Rust kmain(hartid=0, dtb=x19).
    mov  x0, #0             // hartid (CPU 0)
    mov  x1, x19            // DTB pointer stashed from entry x0
    bl   kmain

    // If kmain returns, halt.
3:
    wfi
    b    3b
    "#
);
