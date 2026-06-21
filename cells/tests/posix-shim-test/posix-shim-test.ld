OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS
{
    /* posix-shim-test — short-lived test binary, exits after printing results.
     * 0x2A000000 is above vfs-test (0x28000000) with no current conflicts.
     * All cells need disjoint VA windows in ViCell's single address space. */
    . = 0x2A000000;

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
