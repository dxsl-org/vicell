OUTPUT_ARCH(i386:x86-64)
ENTRY(_start)

SECTIONS
{
    /* VFS user-space base — x86_64.
     * 0x1200_0000 (288 MB): mirrors vfs.ld (RISC-V) to keep the VA map consistent
     * across architectures. Well above init (0x0040_0000) and MMIO,
     * and below shell (0x0800_0000) and the canonical user-space limit. */
    . = 0x0000000012000000;

    .text : {
        *(.text.boot)
        *(.text .text.*)
    }

    .rodata : ALIGN(4096) {
        *(.rodata .rodata.*)
    }

    .data : ALIGN(4096) {
        *(.data .data.*)
        *(.data.rel.ro .data.rel.ro.*)
        *(.got .got.* .got.plt)
    }

    .bss : ALIGN(4096) {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        __bss_end = .;
    }

    /* Cell manifest: kernel reads this section to validate spawn-time caps. */
    __ViCell_manifest : ALIGN(4096) {
        KEEP(*(__ViCell_manifest))
    }

    /* Syscall allowlist emitted by api::declare_syscalls!(). */
    __ViCell_syscalls : ALIGN(4096) {
        KEEP(*(__ViCell_syscalls))
    }
}
