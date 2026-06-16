/* AArch64 bare-metal entry point for the silo guest.
 *
 * The VMM drops the CPU in EL1 (or EL2 if H-ext is active) with the PC
 * pointing at 0x4000_0000 (_start).  MMU is off; Stage-2 translation is
 * managed by the host VMM which pre-maps the mailbox at IPA 0x4000_3000.
 *
 * Sequence:
 *   1. Set stack pointer to _stack_top (label emitted by linker script).
 *   2. Zero the .bss segment (__bss_start .. __bss_end).
 *   3. Branch to silo_main (never returns).
 */

.global _start
.section .text.entry
.type _start, @function
_start:
    /* ── 1. Initialise stack ─────────────────────────────────────────── */
    ldr x0, =_stack_top
    mov sp, x0

    /* ── 2. Zero BSS ─────────────────────────────────────────────────── */
    ldr x1, =__bss_start
    ldr x2, =__bss_end
    cmp x1, x2
    bge .Lbss_done
.Lbss_loop:
    str xzr, [x1], #8
    cmp x1, x2
    blt .Lbss_loop
.Lbss_done:

    /* ── 3. Enter Rust ───────────────────────────────────────────────── */
    bl  silo_main

    /* Should never reach here; spin to avoid undefined behaviour. */
.Lhalt:
    wfi
    b   .Lhalt
