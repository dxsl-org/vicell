OUTPUT_ARCH(aarch64)
ENTRY(_start)

SECTIONS
{
    /* Shell user-space base address — AArch64 ARM virt.
     * 0x0B000000 (176 MB): above VirtIO MMIO (0x0A00_4000) and GIC (0x0800_0000..0x0900_0000).
     * Same range is free on RISC-V (below PLIC at 0x0C000000). */
    . = 0x0B000000;

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
