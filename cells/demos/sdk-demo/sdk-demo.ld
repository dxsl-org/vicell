OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS
{
    /* sdk-demo VA base — RISC-V SV39.
     * 0x38000000 (896 MB): above can-demo (0x36000000). */
    . = 0x38000000;

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

    __ViCell_syscalls : ALIGN(8) {
        KEEP(*(__ViCell_syscalls))
    }
}
