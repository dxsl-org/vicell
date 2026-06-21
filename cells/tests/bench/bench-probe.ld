OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS
{
    /* bench-probe user-space base address: 0x19000000.
     * Distinct from bench (0x18000000–0x18806000) so both binaries can
     * coexist in the SAS page table while the orchestrator runs probe/load
     * children concurrently. */
    . = 0x19000000;

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
