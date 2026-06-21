OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS
{
    /* net-tools binaries (ping/curl/nc/wget/httpd/mqtt) — spawned from the
     * shell while long-lived cells are resident, so the base must not overlap
     * any live cell or kernel/MMIO mapping. 0x2600_0000 sits above the highest
     * cell base (robot-dashboard 0x2200_0000, end ~0x2281_1038).
     *
     * Without this script the toolchain default base is 0x10000, which the
     * loader's overwrite-guard rejects (VA already mapped) — every net tool
     * then failed to spawn as "command not found". */
    . = 0x26000000;

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
