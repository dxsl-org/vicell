OUTPUT_ARCH(i386:x86-64)
ENTRY(_start)

SECTIONS
{
    /* x86_64 user-space base — 4 MB, matches RISC-V/AArch64 convention.
     * Canonical user VA range: 0x0000_0000_0000_0000 – 0x0000_7FFF_FFFF_FFFF.
     * elf.rs USER_VADDR_MAX = 0xFFFF_FFFF caps PT_LOAD addresses (32-bit safe). */
    . = 0x0000000000400000;

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

    __ViCell_manifest : ALIGN(4096) {
        KEEP(*(__ViCell_manifest))
    }

    /* syscall allowlist emitted by api::declare_syscalls!().
     * Must be a named output section so the kernel's get_section() can find it. */
    __ViCell_syscalls : ALIGN(4096) {
        KEEP(*(__ViCell_syscalls))
    }
}
