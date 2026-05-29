OUTPUT_ARCH(aarch64)
ENTRY(_start)

SECTIONS {
    /* QEMU virt loads kernel at 0x40080000 by default. */
    . = 0x40080000;

    .text.boot : {
        KEEP(*(.text.boot))
    }

    .text.vectors : ALIGN(2048) {
        KEEP(*(.text.vectors))
    }

    .text : ALIGN(4096) {
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
    .bss.stack : ALIGN(4096) {
        . = . + 0x10000;
        __stack_top = .;
    }
}
