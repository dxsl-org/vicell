OUTPUT_ARCH(i386:x86-64)
ENTRY(_start)

SECTIONS {
    /* Higher-half kernel: Limine maps the kernel here. */
    . = 0xFFFFFFFF80000000;

    /* R E — executable code. */
    .text : {
        KEEP(*(.text.boot))
        *(.text .text.*)
    }

    /* R — read-only data, isolated on its own page(s) from .text (R E). */
    .rodata : ALIGN(4096) {
        *(.rodata .rodata.*)
    }

    /* RW — Limine request block.  All three input sections are merged here
     * so they form ONE RW LOAD segment (no different-permission pages).
     * The bootloader writes back response pointers and the base-revision into
     * this region, so it MUST be writable.  Order is preserved so Limine
     * can scan from start_marker → requests → end_marker correctly. */
    .requests : ALIGN(4096) {
        KEEP(*(.requests_start_marker))
        KEEP(*(.requests))
        KEEP(*(.requests_end_marker))
    }

    /* RW — initialised data.  .got is explicitly placed here to prevent LLD
     * from inserting it as an orphan between request sections, which would
     * create a separate LOAD segment on the same page and trigger Limine's
     * "different permissions sharing a page" panic. */
    .data : ALIGN(4096) {
        *(.data .data.*)
        *(.sdata .sdata.*)
        *(.got .got.*)
    }

    .bss : ALIGN(4096) {
        __bss_start = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
        __bss_end = .;
    }

    /* 64 KiB kernel stack */
    .stack : ALIGN(4096) {
        . = . + 0x10000;
        __stack_top = .;
    }
}
