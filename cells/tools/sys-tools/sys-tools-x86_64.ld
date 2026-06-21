OUTPUT_ARCH(i386:x86-64)
ENTRY(_start)

SECTIONS
{
    /* sys-tools binaries on x86_64 QEMU q35.
     * 0x2A00_0000 (704 MiB): matches sys-tools.ld (RISC-V) and sys-tools-arm64.ld
     * (which uses 0x3000_0000) convention — all above MMIO and long-lived cells.
     *
     * All sys-tools binaries share this base because they are transient: each one
     * spawns, runs, and exits before the next is invoked, so the VA is reclaimed
     * by the zombie reaper between uses. */
    . = 0x000000002A000000;

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
