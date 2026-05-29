OUTPUT_ARCH(i386:x86-64)
ENTRY(_start)

SECTIONS {
    /* Higher-half kernel: Limine maps kernel here. */
    . = 0xFFFFFFFF80000000;

    .text : {
        KEEP(*(.text.boot))
        *(.text .text.*)
    }

    .rodata : ALIGN(4096) {
        *(.rodata .rodata.*)
    }

    .data : ALIGN(4096) {
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

    /* 64KB kernel stack */
    .stack : ALIGN(4096) {
        . = . + 0x10000;
        __stack_top = .;
    }
}
