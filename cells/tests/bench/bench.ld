OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS
{
    /* Benchmark suite user-space base address.
     * Every cell MUST occupy a disjoint VA window AND avoid the kernel's MMIO
     * identity maps (CLINT 0x0200_0000-0x0201_0000, PLIC 0x0C00_0000-0x1000_0000,
     * UART 0x1000_0000-0x1001_0000), because ViCell maps everything into one shared
     * KERNEL_ROOT page table (SAS) — a cell's segments silently overwrite whatever
     * PTE is already there. Two bugs this base avoids: (1) the old default LLD
     * layout put bench's multi-MB writable segment across vaddr 0x400000 and
     * clobbered init's .text (scause=0xc fault); (2) the old fixed base 0x0C00_0000
     * sat inside the PLIC window (clobbering interrupt-controller MMIO) and even
     * collided with lua's base. New base is in the empty region above all MMIO
     * (>=0x1001_0000) and below RAM (0x8000_0000), distinct from lua/micropython. */
    . = 0x18000000;

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
