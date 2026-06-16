OUTPUT_ARCH(aarch64)
ENTRY(_start)

SECTIONS
{
    /* Security Silo service — AArch64.
     * 0x1E000000 (480 MB): between hypervisor (0x1C000000) and robot-demo (0x20000000).
     * Silo cell is tiny (~50 KB) — no large BSS budget needed. */
    . = 0x1E000000;

    .text : {
        *(.text.boot)
        *(.text .text.*)
    }

    .rodata : ALIGN(4096) {
        *(.rodata .rodata.*)
    }

    /* ViCell manifest sections — must be page-aligned off .bss (Law: linker gotcha). */
    .vicell_manifest : ALIGN(4096) {
        KEEP(*(.vicell_manifest))
    }
    __ViCell_manifest = ADDR(.vicell_manifest);

    .vicell_syscalls : ALIGN(8) {
        KEEP(*(.vicell_syscalls))
    }
    __ViCell_syscalls = ADDR(.vicell_syscalls);

    .data : ALIGN(4096) {
        *(.data .data.*)
    }

    .bss : ALIGN(4096) {
        *(.bss .bss.*)
        *(COMMON)
    }

    /DISCARD/ : {
        *(.comment)
        *(.eh_frame)
        *(.note.*)
    }
}
