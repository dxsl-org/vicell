OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS
{
    /* DOOM cell VA base — RISC-V SV39 user space.
     * 0x42000000 (1056 MB): first free slot above hypervisor-test (0x40000000).
     * Leaves 64 MB headroom before the next natural boundary. */
    . = 0x42000000;

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
