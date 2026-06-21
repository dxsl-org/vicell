OUTPUT_ARCH(i386:x86-64)
ENTRY(_start)

SECTIONS
{
    /* Shell user-space base — x86_64.
     * 128 MB: matches shell.ld (RISC-V) and shell-arm64.ld (AArch64) convention.
     * Well above VFS (0x1200_0000 + ~16MB) and init (0x0040_0000),
     * and below the canonical user-space upper limit (0x0000_7FFF_FFFF_FFFF). */
    . = 0x0000000008000000;

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
