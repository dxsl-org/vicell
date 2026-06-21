OUTPUT_ARCH(aarch64)
ENTRY(_start)

SECTIONS
{
    /* sys-tools binaries on AArch64 ARM virt.
     * 0x30000000 (768 MiB): above sensor-demo (0x2E000000) and all VirtIO / GIC MMIO.
     * Same transient sharing rationale as sys-tools.ld. */
    . = 0x30000000;

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
        *(.sdata .sdata.*)
    }

    .bss : ALIGN(4096) {
        __bss_start = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
        __bss_end = .;
    }

    __ViCell_manifest : ALIGN(4096) {
        KEEP(*(__ViCell_manifest))
    }
}
