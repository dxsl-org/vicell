OUTPUT_ARCH(aarch64)
ENTRY(_start)

SECTIONS
{
    /* periph-demo VA base — AArch64 ARM virt.
     * 0x22000000 (544 MB): above robot-demo (0x20000000), compositor (0x1A000000),
     * VFS (0x12000000), shell (0x0B000000), and all MMIO (ends ~0x0A010000). */
    . = 0x22000000;

    .text : {
        *(.text.boot)
        *(.text .text.*)
    }

    .rodata : ALIGN(4096) {
        *(.rodata .rodata.*)
    }

    .data : ALIGN(4096) {
        *(.data .data.*)
        *(.data.rel.ro .data.rel.ro.*)
        *(.got .got.* .got.plt)
    }

    .bss : ALIGN(4096) {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        __bss_end = .;
    }

    __ViCell_manifest : ALIGN(4096) {
        KEEP(*(__ViCell_manifest))
    }
}
