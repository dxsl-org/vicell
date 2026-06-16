//! AArch64 boot entry point.
//!
//! Two EL paths share a single entry:
//!   - EL2 (QEMU `virtualization=on`): `.el2_init` stays at EL2, sets HCR_EL2,
//!     calls `el2_mark_active`, then `kmain`.
//!   - EL1 (default QEMU without `-machine virt,virtualization=on`): `.el1_entry`
//!     runs the existing EL1 setup and calls `kmain` without marking EL2.
//!
//! Both paths share BSS clearing and stack setup; the EL1 path does those steps
//! itself while the EL2 path folds them into `.el2_init`.

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
    b.eq .el2_init
    b .el1_entry             // Already in EL1

.el2_init:
    // F2: set HCR_EL2 = RW(1<<31) | TGE(1<<27) FIRST.
    // TGE routes EL0 exceptions to VBAR_EL2 — required for Cell SVCs at EL2 host.
    // RW ensures any future EL1 guest runs AArch64 (also harmless now).
    // SAFETY: we are at EL2; HCR_EL2 is EL2-private.
    mov x0, #(1 << 31)
    orr x0, x0, #(1 << 27)
    msr hcr_el2, x0
    isb

    // Enable FP/SIMD at EL2 host (CPTR_EL2=0 disables all traps).
    msr cptr_el2, xzr
    isb

    // Set SP_EL2 stack.
    adrp x0, __stack_top
    add  x0, x0, :lo12:__stack_top
    mov  sp, x0

    // Clear BSS.
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
    // UART sentinel 'E': confirms EL2 init path was taken.
    // Visible even if kmain hangs before UART is fully initialised.
    mov  x0, #0x09000000
    mov  w1, #0x45          // ASCII 'E'
    strb w1, [x0]

    // Mark EL2_ACTIVE = true and jump to kmain.
    bl   el2_mark_active
    mov  x0, #0             // hartid = 0
    mov  x1, x19            // DTB pointer
    bl   kmain

    // If kmain returns, halt.
3:
    wfi
    b    3b

.el1_entry:
    // Enable FP/SIMD in EL1 and EL0 (CPACR_EL1.FPEN = 0b11).
    // Without this, any FP/SIMD instruction traps with EC=0x07.
    mov x0, #(3 << 20)
    msr cpacr_el1, x0
    isb

    // Set up initial stack at __stack_top (defined in linker script).
    adrp x0, __stack_top
    add  x0, x0, :lo12:__stack_top
    mov  sp, x0

    // Clear BSS section.
    adrp x0, __bss_start
    add  x0, x0, :lo12:__bss_start
    adrp x1, __bss_end
    add  x1, x1, :lo12:__bss_end
4:
    cmp  x0, x1
    b.hs 5f
    str  xzr, [x0], #8
    b    4b
5:
    // Jump to Rust kmain(hartid=0, dtb=x19).
    mov  x0, #0             // hartid (CPU 0)
    mov  x1, x19            // DTB pointer stashed from entry x0
    bl   kmain

    // If kmain returns, halt.
6:
    wfi
    b    6b
    "#
);
