# S-mode Trap Entry for RISC-V 64-bit (ViOS)
# This is the entry point for all traps (interrupts, exceptions, syscalls)
# Uses sscratch for kernel stack swap per RISC-V convention

.section .text
.global __trap_entry
.global vi_set_sscratch
.align 4

# Trap Frame Layout (matches ViTrapFrame struct):
# regs[32] = 32 * 8 = 256 bytes
# sstatus  = 8 bytes
# sepc     = 8 bytes
# stval    = 8 bytes
# scause   = 8 bytes
# Total    = 288 bytes

.equ TRAP_FRAME_SIZE, 288

__trap_entry:
    # === STEP 1: Stack Swap using sscratch ===
    # sscratch holds kernel stack pointer when in U-mode
    # When trap from U-mode: swap sp <-> sscratch (get kernel stack)
    # When trap from S-mode: sscratch=0, skip swap (already on kernel stack)
    
    csrrw sp, sscratch, sp      # Atomically swap sp and sscratch
    
    # Check if we came from U-mode by checking if sscratch (now sp) was zero
    # If sp == 0, we came from S-mode, need to restore original sp
    bnez sp, .from_user_mode
    
    # From S-mode: restore original sp from sscratch
    # From S-mode: restore original sp from sscratch AND restore sscratch to 0
    csrrw sp, sscratch, sp      # Swap back: sp=original, sscratch=0
    j .continue_save
    
.from_user_mode:
    # sp now contains kernel stack, sscratch contains user sp
    # Continue with kernel stack
    
.continue_save:
    # === STEP 2: Allocate trap frame on stack ===
    addi sp, sp, -TRAP_FRAME_SIZE
    
    # === STEP 3: Save all general purpose registers ===
    # x0 is always 0, skip it but keep slot for alignment
    sd x1,   1*8(sp)    # ra
    # x2 (sp) saved below after we calculate user sp
    sd x3,   3*8(sp)    # gp
    sd x4,   4*8(sp)    # tp  (cell's tp saved here; kernel tp restored below)
    sd x5,   5*8(sp)    # t0  (save early so t0 is free for the tp restore below)

    # === Phase 02: restore kernel tp from per-hart slot ===
    # HART_LOCAL_TP_ADDR holds &HART_LOCALS[hart_id] (set by hart_local::install).
    # Before install() it is 0 — current_cell_id() null-guards that case safely.
    # `lla` generates auipc+addi (PC-relative) — correct for PIE at any load bias.
    .option push
    .option norelax
    lla  t0, HART_LOCAL_TP_ADDR   # t0 = &HART_LOCAL_TP_ADDR (a static usize)
    .option pop
    ld   tp, 0(t0)                # tp = *HART_LOCAL_TP_ADDR = &HART_LOCALS[hart_id]
    sd x6,   6*8(sp)    # t1
    sd x7,   7*8(sp)    # t2
    sd x8,   8*8(sp)    # s0/fp
    sd x9,   9*8(sp)    # s1
    sd x10, 10*8(sp)    # a0
    sd x11, 11*8(sp)    # a1
    sd x12, 12*8(sp)    # a2
    sd x13, 13*8(sp)    # a3
    sd x14, 14*8(sp)    # a4
    sd x15, 15*8(sp)    # a5
    sd x16, 16*8(sp)    # a6
    sd x17, 17*8(sp)    # a7
    sd x18, 18*8(sp)    # s2
    sd x19, 19*8(sp)    # s3
    sd x20, 20*8(sp)    # s4
    sd x21, 21*8(sp)    # s5
    sd x22, 22*8(sp)    # s6
    sd x23, 23*8(sp)    # s7
    sd x24, 24*8(sp)    # s8
    sd x25, 25*8(sp)    # s9
    sd x26, 26*8(sp)    # s10
    sd x27, 27*8(sp)    # s11
    sd x28, 28*8(sp)    # t3
    sd x29, 29*8(sp)    # t4
    sd x30, 30*8(sp)    # t5
    sd x31, 31*8(sp)    # t6
    
    # === STEP 4: Save CSRs ===
    csrr t0, sstatus
    sd t0, 32*8(sp)     # sstatus
    csrr t0, sepc
    sd t0, 33*8(sp)     # sepc
    csrr t0, stval
    sd t0, 34*8(sp)     # stval
    csrr t0, scause
    sd t0, 35*8(sp)     # scause
    
    # === STEP 5: Save user sp ===
    # If from U-mode, user sp is in sscratch
    # If from S-mode, user sp is current sp + TRAP_FRAME_SIZE
    csrr t0, sstatus
    andi t0, t0, 0x100          # Check SPP bit (bit 8)
    bnez t0, .save_smode_sp
    
    # From U-mode: user sp is in sscratch
    csrr t0, sscratch
    sd t0, 2*8(sp)
    j .call_handler
    
.save_smode_sp:
    # From S-mode: calculate original sp
    addi t0, sp, TRAP_FRAME_SIZE
    sd t0, 2*8(sp)
    
.call_handler:
    # === STEP 6: Call Rust trap handler ===
    mv a0, sp               # a0 = &ViTrapFrame (borrowed, not owned)
    call vi_trap_handler    # Rust function with Vi prefix
    
    # === STEP 7: Restore CSRs ===
    .global __trap_exit
__trap_exit:
    ld t0, 32*8(sp)
    csrw sstatus, t0
    ld t0, 33*8(sp)
    csrw sepc, t0
    
    # === STEP 8: Check if returning to U-mode ===
    # If SPP=0, restore sscratch with kernel stack for next trap
    csrr t0, sstatus
    andi t0, t0, 0x100
    bnez t0, .restore_smode
    
    # Returning to U-mode: set sscratch = kernel stack top
    addi t0, sp, TRAP_FRAME_SIZE
    csrw sscratch, t0
    
.restore_smode:
    # === STEP 9: Restore general purpose registers ===
    ld x1,   1*8(sp)    # ra
    # Skip x2 (sp) - restore last
    ld x3,   3*8(sp)    # gp
    ld x4,   4*8(sp)    # tp
    ld x5,   5*8(sp)    # t0
    ld x6,   6*8(sp)    # t1
    ld x7,   7*8(sp)    # t2
    ld x8,   8*8(sp)    # s0/fp
    ld x9,   9*8(sp)    # s1
    ld x10, 10*8(sp)    # a0
    ld x11, 11*8(sp)    # a1
    ld x12, 12*8(sp)    # a2
    ld x13, 13*8(sp)    # a3
    ld x14, 14*8(sp)    # a4
    ld x15, 15*8(sp)    # a5
    ld x16, 16*8(sp)    # a6
    ld x17, 17*8(sp)    # a7
    ld x18, 18*8(sp)    # s2
    ld x19, 19*8(sp)    # s3
    ld x20, 20*8(sp)    # s4
    ld x21, 21*8(sp)    # s5
    ld x22, 22*8(sp)    # s6
    ld x23, 23*8(sp)    # s7
    ld x24, 24*8(sp)    # s8
    ld x25, 25*8(sp)    # s9
    ld x26, 26*8(sp)    # s10
    ld x27, 27*8(sp)    # s11
    ld x28, 28*8(sp)    # t3
    ld x29, 29*8(sp)    # t4
    ld x30, 30*8(sp)    # t5
    ld x31, 31*8(sp)    # t6
    
    # === STEP 10: Restore sp and return ===
    ld sp,   2*8(sp)    # Restore user/kernel sp
    
    sret

# Helper function to set sscratch (called from Rust before entering userspace)
# fn vi_set_sscratch(kernel_stack_top: usize)
vi_set_sscratch:
    csrw sscratch, a0
    ret
