OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS
{
    /* VFS test cell user-space base — RISC-V SV39 user VA.
     * 0x28000000 is above net-tools (0x26000000) with no conflicts in the
     * current cell map.  Every cell needs a disjoint VA window because ViCell
     * maps all cells into one shared KERNEL_ROOT page table (SAS). */
    . = 0x28000000;

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
