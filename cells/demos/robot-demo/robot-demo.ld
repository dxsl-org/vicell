OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS
{
    /* robot-demo user-space base address: 0x20000000 (512 MB virtual).
     * Avoids: kernel MMIO (ends ~0x10010000), vfs (0x12000000-0x12450000),
     * bench (0x18000000-0x18600000), and all other cells. */
    . = 0x20000000;

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
        *(.data.rel.ro .data.rel.ro.*)
        *(.got .got.* .got.plt)
        *(.sdata .sdata.*)
    }

    .bss : ALIGN(4096) {
        __bss_start = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
        __bss_end = .;
    }

    __ViCell_manifest : ALIGN(4096) {
        KEEP(*(__ViCell_manifest))
    }
}
