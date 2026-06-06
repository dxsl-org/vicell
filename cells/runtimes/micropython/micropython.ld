OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS
{
    /* MicroPython runtime cell. Must avoid the kernel MMIO identity maps and every
     * other cell's window (SAS — one shared page table). Old base 0x0E000000 sat
     * inside the PLIC window (0x0C00_0000-0x1000_0000), silently clobbering
     * interrupt-controller MMIO. Placed in the empty region above all MMIO
     * (>=0x1001_0000), below RAM (0x8000_0000), distinct from bench/lua. */
    . = 0x20000000;

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
}
