OUTPUT_ARCH(aarch64)
ENTRY(_start)

SECTIONS
{
    /* silo-test VA base — AArch64 ARM virt.
     * 0x3C000000 (960 MiB): above input-test (0x3A000000), below RAM (0x40000000). */
    . = 0x3C000000;

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

    __ViCell_syscalls : ALIGN(8) {
        KEEP(*(__ViCell_syscalls))
    }
}
