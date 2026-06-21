OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS
{
    /* User-space base address for RISC-V SV39.
     * User VA range: 0x0000_0000 – 0x7FFF_FFFF (lower half of 39-bit VA).
     * 4 MB base keeps cells well below the kernel at 0x80200000. */
    . = 0x00400000;
    
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
