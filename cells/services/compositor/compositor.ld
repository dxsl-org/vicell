OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS
{
    /* Compositor service user-space base address.
     * Placed at 160MB — above shell (128MB + ~7MB BSS = ~135MB).
     * The compositor holds a 1024x768 framebuffer = 3MB in BSS. */
    . = 0x0A000000;

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
