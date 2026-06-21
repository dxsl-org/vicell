OUTPUT_ARCH(riscv)
ENTRY(_start)

MEMORY
{
    /* 256MB RAM starting at 0x80200000 (OpenSBI jump address) */
    ram (wxa) : ORIGIN = 0x80200000, LENGTH = 256M
}

SECTIONS
{
    /* Kernel code and data
     * KEEP(.text.boot) is mandatory: the bootloader (Limine / OpenSBI jump)
     * enters at _start which lives in .text.boot. Without KEEP, --gc-sections
     * may drop it under release LTO and the kernel never boots.
     */
    .text : {
        KEEP(*(.text.boot))
        *(.text .text.*)
    } >ram

    .rodata : ALIGN(4096) {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    } >ram

    /* Dynamic relocations for the PIE kernel.
     * The kernel is linked -pie (ET_DYN) so Limine can apply KASLR. Under a
     * direct QEMU `-bios default -kernel <elf>` boot there is no dynamic loader,
     * so _start self-applies these R_RISCV_RELATIVE entries (see rv64/boot.rs).
     * __rela_dyn_start/__rela_dyn_end bound the Elf64_Rela array (24 bytes each).
     * SHF_ALLOC ensures it lands in a PT_LOAD segment so it is present at boot.
     */
    .rela.dyn : ALIGN(8) {
        __rela_dyn_start = .;
        *(.rela .rela.*)
        __rela_dyn_end = .;
    } >ram

    .data : ALIGN(4096) {
        . = ALIGN(16);
        __global_pointer$ = . + 0x800;
        *(.data .data.*)
        *(.sdata .sdata.*)
    } >ram

    .bss : ALIGN(4096) {
        __bss_start = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
        __bss_end = .;
    } >ram

    /* Kernel Stack (64KB) - NOLOAD keeps 64KB of zeros out of the kernel image. */
    .kernel_stack (NOLOAD) : ALIGN(4096) {
        . = ALIGN(4096);
        __stack_bottom = .;
        *(.stack)
        . = . + 0x10000;
        __stack_top = .;
    } >ram

    /DISCARD/ : {
        *(.eh_frame)
    }
}
