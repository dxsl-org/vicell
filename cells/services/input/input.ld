OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS
{
    /* Input service user-space base address.
     * Placed at 64MB — between VFS (32MB) and config (8MB ends ~12MB).
     * Input cell BSS is small (~64KB for modifier state + dispatcher). */
    . = 0x04000000;

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
