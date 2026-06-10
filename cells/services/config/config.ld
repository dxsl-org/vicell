OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS
{
    /* Config service user-space base address.
     * Placed above init (0x400000 + ~23KB) with enough gap for init's heap.
     * 8 MB gives config up to 4 MB of BSS (cell heap) before reaching 12 MB.
     * Shell is at 0x08000000 (128 MB), so no overlap. */
    . = 0x00800000;

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
