/// Boot module - Assembly entry point and early initialization
///
/// This module handles the transition from bootloader to Rust code
use core::arch::global_asm;

// Secondary-hart entry point.
// Hart 0's `_start` runs self-relocation + BSS clear and MUST NOT be re-entered.
// Secondary harts start here — bare entry, no relocation, no BSS clear.
// a0 = hart_id (set by OpenSBI HSM per SBI spec §9.1.1)
// a1 = opaque = kernel stack top (our convention, passed in sbi_hart_start)
global_asm!(
    r#"
    .section .text
    .global _secondary_entry
    .align 2
_secondary_entry:
    .option push
    .option norelax
    lla gp, __global_pointer$   # restore GP (PC-relative, GOT-independent)
    .option pop
    mv  sp, a1          # kernel stack = stack_top from SBI opaque
    mv  tp, zero        # Phase 01 placeholder; Phase 02 sets real HartLocal ptr
    call smp_hart_entry # fn(hart_id: usize) -> !   (a0 = hart_id from SBI)
.secondary_halt:
    wfi
    j .secondary_halt
    "#
);

// Assembly boot code
global_asm!(
    r#"
    .section .text.boot
    .global _start
_start:
    # 1. Disable interrupts
    csrw sie, zero
    csrw sip, zero

    # CRITICAL: every address below uses `lla` (load-local-address), which
    # always expands to PC-relative `auipc`+`addi`. Plain `la` under PIC
    # codegen (relocation-model=pic) expands to a GOT-indirect load
    # (`auipc`+`ld [GOT]`), and the GOT slots are exactly the un-relocated
    # NULL pointers we are about to fix — a chicken-and-egg that left gp/sp/t*
    # all zero and faulted before the first UART byte. `lla` is GOT-independent
    # and correct at any load bias.
    .option push
    .option norelax

    # 2. Initialize global pointer (gp) — PC-relative, GOT-independent.
    lla gp, __global_pointer$

    # 3. Initialize thread pointer (tp) - Clear it for now
    mv tp, zero

    # 4. Set up stack pointer
    lla sp, __stack_top

    # 5. Self-apply R_RISCV_RELATIVE relocations.
    #    The kernel is linked -pie (ET_DYN). Under Limine the bootloader applies
    #    these; under direct QEMU `-kernel` there is no loader, so we apply them
    #    ourselves before any global/GOT slot is dereferenced (the first println!
    #    reads a .data/.got pointer that is otherwise an un-relocated NULL).
    #
    #    Load bias (slide) = runtime(_start) - link_origin(0x80200000).
    #    Each Elf64_Rela is 24 bytes: r_offset(8) r_info(8) r_addend(8).
    #    For R_RISCV_RELATIVE: *(slide + r_offset) = slide + r_addend.
    #    Uses only t0-t6, preserves a0/a1 (hartid/dtb) for kmain. At slide=0
    #    (direct -kernel) this writes the identity-correct absolute addresses;
    #    under Limine (slide!=0) it is also correct.
    lla  t0, _start            # t0 = runtime address of _start
    li   t1, 0x80200000        # t1 = link-time origin
    sub  t2, t0, t1            # t2 = slide (load bias); 0 for direct -kernel
    lla  t3, __rela_dyn_start  # t3 = &rela[0]
    lla  t4, __rela_dyn_end    # t4 = end
6:
    bgeu t3, t4, 7f            # done when t3 >= end
    ld   t5, 8(t3)             # t5 = r_info
    li   t6, 3                 # R_RISCV_RELATIVE == 3 (full r_info, sym idx 0)
    bne  t5, t6, 8f            # skip non-RELATIVE entries (none expected)
    ld   t5, 0(t3)             # t5 = r_offset (link-relative)
    add  t5, t5, t2            # t5 = slot runtime addr = slide + r_offset
    ld   t6, 16(t3)            # t6 = r_addend (link-relative target)
    add  t6, t6, t2            # t6 = slide + r_addend
    sd   t6, 0(t5)             # *slot = relocated value
8:
    addi t3, t3, 24            # advance to next Elf64_Rela
    j    6b
7:

    # 6. Clear BSS section (PC-relative bounds).
    lla t0, __bss_start
    lla t1, __bss_end
1:
    bgeu t0, t1, 2f
    sd zero, 0(t0)
    addi t0, t0, 8
    j 1b
2:
    .option pop

    # Jump to Rust entry point (defined in kernel)
    call kmain
    
    # If kmain returns, halt
3:
    wfi
    j 3b
    "#
);
