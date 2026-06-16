/* x86_32 linker script — multiboot1 via QEMU -kernel, load at 1 MiB */
OUTPUT_ARCH(i386)
ENTRY(_start)

SECTIONS {
    . = 0x00100000;   /* 1 MiB — standard multiboot load address */

    /* Multiboot header MUST appear in the first 8 KiB of the binary. */
    .multiboot : {
        KEEP(*(.multiboot))
    }

    .text.boot : { KEEP(*(.text.boot)) }
    .text       : { *(.text .text.*) }

    .rodata     : ALIGN(4096) {
        *(.rodata .rodata.*)
    }

    .data       : ALIGN(4096) {
        *(.data .data.*)
        *(.got .got.plt)
    }

    .bss        : ALIGN(4096) {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        __bss_end = .;
    }

    /* Kernel stack — not loaded, allocated in BSS region. */
    .stack (NOLOAD) : ALIGN(4096) {
        . = . + 0x8000;   /* 32 KiB */
        __stack_top = .;
    }

    /DISCARD/ : {
        *(.comment)
        *(.eh_frame .eh_frame_hdr)
        *(.note.*)
    }
}
