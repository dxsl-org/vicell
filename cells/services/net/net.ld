OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS
{
    /* Net service user-space base address.
     * Placed at 96MB — between input service (64MB) and shell (128MB).
     * The net service may have large BSS for smoltcp socket buffers. */
    . = 0x06000000;

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
