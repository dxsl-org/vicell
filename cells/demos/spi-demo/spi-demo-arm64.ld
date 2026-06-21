OUTPUT_ARCH(aarch64)
ENTRY(_start)

SECTIONS
{
    /* spi-demo VA base — AArch64 ARM virt.
     * 0x30000000 (768 MB): above sensor-demo (0x2E000000), srv-test (0x2C000000),
     * posix-shim-test (0x2A000000), and all other occupied bases. */
    . = 0x30000000;

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
