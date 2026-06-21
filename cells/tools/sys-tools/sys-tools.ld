OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS
{
    /* sys-tools binaries (ls/cat/echo/ps/kill) — transient debug utilities spawned
     * from the shell one at a time.  0x2A000000 sits above net-tools (0x2600_0000)
     * and all known MMIO / long-lived cell ranges.
     *
     * All sys-tools binaries share this base because they are transient: each one
     * spawns, runs, and exits before the next is invoked, so the VA is reclaimed
     * by the zombie reaper between uses.
     *
     * Without this script the toolchain default base is 0x10000 (LLD) which the
     * loader's overwrite-guard rejects (VA already mapped by init). */
    . = 0x2A000000;

    .text : {
        *(.text.boot)
        *(.text .text.*)
    }

    .rodata : ALIGN(4096) {
        *(.rodata .rodata.*)
    }

    .data : ALIGN(4096) {
        PROVIDE(__global_pointer$ = . + 0x800);
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
