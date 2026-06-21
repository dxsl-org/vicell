OUTPUT_ARCH(i386:x86-64)
ENTRY(_start)

SECTIONS
{
    /* DOOM cell VA base — x86_64.
     * 0x42000000: matches RISC-V/AArch64 slot convention. */
    . = 0x0000000042000000;

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

    __ViCell_syscalls : ALIGN(4096) {
        KEEP(*(__ViCell_syscalls))
    }
}
