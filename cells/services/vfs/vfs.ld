OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS
{
    /* VFS user-space base address — RISC-V SV39 user VA (< 0x8000_0000).
     * MUST avoid the kernel's MMIO identity maps (CLINT 0x0200_0000-0x0201_0000,
     * PLIC 0x0C00_0000-0x1000_0000, UART/VirtIO 0x1000_0000-0x1001_0000): in the
     * single shared SAS page table a cell's segments would silently overwrite
     * those MMIO PTEs. Old base 0x0200_0000 sat inside CLINT. Placed in the empty
     * region above all MMIO (>=0x1001_0000) and below RAM (0x8000_0000). */
    . = 0x12000000;
    
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
        *(.sdata .sdata.*)
    }
    
    .bss : ALIGN(4096) {
        __bss_start = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
        __bss_end = .;
    }

    .vicell_manifest : ALIGN(8) {
        KEEP(*(__ViCell_manifest))
    }
}
