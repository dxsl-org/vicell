OUTPUT_ARCH(aarch64)
ENTRY(_start)

SECTIONS
{
    /* hypervisor-test VA base — AArch64 ARM virt.
     * 0x24000000 (576 MB): above periph-demo (0x22000000), below RAM (0x40000000). */
    . = 0x24000000;

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
