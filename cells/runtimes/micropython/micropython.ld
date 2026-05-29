OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS
{
    /* MicroPython runtime cell.
     * VA 0x0E000000 (224 MB) — above compositor (0x0A000000) and lua (0x0C000000).
     * Leaves 32 MB for BSS / heap before hitting 0x10000000. */
    . = 0x0E000000;

    .text : {
        *(.text.boot)
        *(.text .text.*)
    }

    .rodata : ALIGN(4096) {
        *(.rodata .rodata.*)
    }

    .data : ALIGN(4096) {
        PROVIDE(__global_pointer$ = . + 0x800);
        *(.data .data.*)
        *(.sdata .sdata.*)
    }

    .bss : ALIGN(4096) {
        __bss_start = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
        __bss_end = .;
    }
}
