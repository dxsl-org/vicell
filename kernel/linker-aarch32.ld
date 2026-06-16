/* AArch32 linker script — QEMU ARM virt -kernel, load at 0x40080000 */
OUTPUT_ARCH(arm)
ENTRY(_start)

SECTIONS {
    . = 0x40080000;   /* ARM virt QEMU load address (same as AArch64 virt) */

    .text.boot  : { KEEP(*(.text.boot)) }
    .text        : ALIGN(4) { *(.text .text.*) }

    .rodata      : ALIGN(4096) {
        *(.rodata .rodata.*)
    }

    .data        : ALIGN(4096) {
        *(.data .data.*)
        *(.got .got.plt)
    }

    .bss         : ALIGN(4096) {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        __bss_end = .;
    }

    /* Kernel stack — not loaded, allocated above BSS. */
    .stack (NOLOAD) : ALIGN(4096) {
        . = . + 0x10000;   /* 64 KiB */
        __stack_top = .;
    }

    /DISCARD/ : {
        *(.comment)
        *(.eh_frame .eh_frame_hdr)
        *(.note.*)
        *(.ARM.exidx .ARM.exidx.*)
        *(.ARM.extab .ARM.extab.*)
    }
}
