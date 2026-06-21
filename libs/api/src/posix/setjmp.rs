// SPDX-License-Identifier: MPL-2.0
// setjmp / longjmp — arch-specific naked asm for non-local jumps.
//
// Required by: Lua 5.4 (error recovery via pcall), MicroPython (nlrsetjmp.c), DOOM.
//
// jmp_buf layout: a fixed-size [u64] array storing callee-saved registers.
// Matches the C ABI for each architecture so C headers work without adaptation.

#![allow(unsafe_code)]

// jmp_buf word counts per arch
#[cfg(target_arch = "riscv64")]
pub const JMP_BUF_WORDS: usize = 16; // ra + sp + s0–s11 + fs0–fs1

#[cfg(target_arch = "aarch64")]
pub const JMP_BUF_WORDS: usize = 22; // x19–x30 + sp + d8–d15 + 1 spare

#[cfg(not(any(target_arch = "riscv64", target_arch = "aarch64")))]
pub const JMP_BUF_WORDS: usize = 9; // rbx + rbp + r12–r15 + rsp + rip/fpcw

/// Opaque jmp_buf type — layout is architecture-specific.
#[repr(C)]
pub struct JmpBuf(pub [u64; JMP_BUF_WORDS]);

// ---------------------------------------------------------------------------
// RISC-V 64 (rv64gc)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "riscv64")]
#[unsafe(naked)]
#[no_mangle]
pub unsafe extern "C" fn setjmp(env: *mut JmpBuf) -> i32 {
    // a0 = env; saves ra, sp, s0–s11, fs0–fs1; returns 0 in a0
    //
    // `.option arch, +d` is required: LLVM's naked_asm! assembler does not
    // automatically inherit the 'D' extension from the target triple
    // (riscv64gc) in some nightly versions — explicit opt-in avoids the
    // "instruction requires 'D'" assembler error.
    core::arch::naked_asm!(
        "sd   ra,   0*8(a0)",
        "sd   sp,   1*8(a0)",
        "sd   s0,   2*8(a0)",
        "sd   s1,   3*8(a0)",
        "sd   s2,   4*8(a0)",
        "sd   s3,   5*8(a0)",
        "sd   s4,   6*8(a0)",
        "sd   s5,   7*8(a0)",
        "sd   s6,   8*8(a0)",
        "sd   s7,   9*8(a0)",
        "sd   s8,  10*8(a0)",
        "sd   s9,  11*8(a0)",
        "sd   s10, 12*8(a0)",
        "sd   s11, 13*8(a0)",
        ".option arch, +d",
        "fsd  fs0, 14*8(a0)",
        "fsd  fs1, 15*8(a0)",
        "li   a0, 0",
        "ret",
    )
}

#[cfg(target_arch = "riscv64")]
#[unsafe(naked)]
#[no_mangle]
pub unsafe extern "C" fn longjmp(env: *const JmpBuf, val: i32) -> ! {
    // a0 = env, a1 = val. Restores all callee-saved regs, returns val (or 1 if val==0).
    core::arch::naked_asm!(
        "ld   ra,   0*8(a0)",
        "ld   sp,   1*8(a0)",
        "ld   s0,   2*8(a0)",
        "ld   s1,   3*8(a0)",
        "ld   s2,   4*8(a0)",
        "ld   s3,   5*8(a0)",
        "ld   s4,   6*8(a0)",
        "ld   s5,   7*8(a0)",
        "ld   s6,   8*8(a0)",
        "ld   s7,   9*8(a0)",
        "ld   s8,  10*8(a0)",
        "ld   s9,  11*8(a0)",
        "ld   s10, 12*8(a0)",
        "ld   s11, 13*8(a0)",
        ".option arch, +d",
        "fld  fs0, 14*8(a0)",
        "fld  fs1, 15*8(a0)",
        // a0 = (a1 == 0) ? 1 : a1
        "mv   a0, a1",
        "seqz t0, a0",
        "add  a0, a0, t0",
        "ret",
    )
}

// ---------------------------------------------------------------------------
// AArch64
// ---------------------------------------------------------------------------

#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
#[no_mangle]
pub unsafe extern "C" fn setjmp(env: *mut JmpBuf) -> i32 {
    // x0 = env; saves x19–x30, sp, d8–d15; returns 0 in w0
    core::arch::naked_asm!(
        "stp x19, x20, [x0,  #0*8]",
        "stp x21, x22, [x0,  #2*8]",
        "stp x23, x24, [x0,  #4*8]",
        "stp x25, x26, [x0,  #6*8]",
        "stp x27, x28, [x0,  #8*8]",
        "stp x29, x30, [x0, #10*8]",
        "mov x2,  sp",
        "str x2,  [x0, #12*8]",
        "stp d8,  d9,  [x0, #13*8]",
        "stp d10, d11, [x0, #15*8]",
        "stp d12, d13, [x0, #17*8]",
        "stp d14, d15, [x0, #19*8]",
        "mov w0,  #0",
        "ret",
    )
}

#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
#[no_mangle]
pub unsafe extern "C" fn longjmp(env: *const JmpBuf, val: i32) -> ! {
    // x0 = env, w1 = val. Restores state, "returns" val (or 1) at setjmp site.
    core::arch::naked_asm!(
        "ldp x19, x20, [x0,  #0*8]",
        "ldp x21, x22, [x0,  #2*8]",
        "ldp x23, x24, [x0,  #4*8]",
        "ldp x25, x26, [x0,  #6*8]",
        "ldp x27, x28, [x0,  #8*8]",
        "ldp x29, x30, [x0, #10*8]",
        "ldr x2,       [x0, #12*8]",
        "mov sp,  x2",
        "ldp d8,  d9,  [x0, #13*8]",
        "ldp d10, d11, [x0, #15*8]",
        "ldp d12, d13, [x0, #17*8]",
        "ldp d14, d15, [x0, #19*8]",
        // w0 = (w1 == 0) ? 1 : w1
        "cmp w1, #0",
        "csinc w0, w1, wzr, ne",
        "ret",
    )
}

// ---------------------------------------------------------------------------
// x86_64 (System V AMD64 ABI)
// ---------------------------------------------------------------------------
// Layout: [rbx, rbp, r12, r13, r14, r15, rsp, rip, _spare]
//          0    8    16   24   32   40   48   56   64

#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
#[no_mangle]
pub unsafe extern "C" fn setjmp(env: *mut JmpBuf) -> i32 {
    // rdi = env
    // [rsp] = return address; rsp+8 = caller's rsp after setjmp returns
    core::arch::naked_asm!(
        "mov [rdi +  0], rbx",
        "mov [rdi +  8], rbp",
        "mov [rdi + 16], r12",
        "mov [rdi + 24], r13",
        "mov [rdi + 32], r14",
        "mov [rdi + 40], r15",
        "lea rax, [rsp + 8]",   // caller rsp (setjmp will have ret'd)
        "mov [rdi + 48], rax",
        "mov rax, [rsp]",       // return address
        "mov [rdi + 56], rax",
        "xor eax, eax",
        "ret",
    )
}

#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
#[no_mangle]
pub unsafe extern "C" fn longjmp(env: *const JmpBuf, val: i32) -> ! {
    // rdi = env, esi = val; restores all callee-saved regs, jumps to saved rip
    core::arch::naked_asm!(
        "mov rbx, [rdi +  0]",
        "mov rbp, [rdi +  8]",
        "mov r12, [rdi + 16]",
        "mov r13, [rdi + 24]",
        "mov r14, [rdi + 32]",
        "mov r15, [rdi + 40]",
        "mov rsp, [rdi + 48]",
        "mov rdx, [rdi + 56]",  // saved rip
        "mov eax, esi",          // return value = val
        "test eax, eax",
        "jnz 2f",
        "inc eax",               // val == 0 → return 1
        "2:",
        "jmp rdx",               // indirect jump to saved rip
    )
}

// ---------------------------------------------------------------------------
// wasm32 + other arches: stub only
// ---------------------------------------------------------------------------

#[cfg(not(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "x86_64")))]
#[no_mangle]
pub unsafe extern "C" fn setjmp(_env: *mut JmpBuf) -> i32 { 0 }

#[cfg(not(any(target_arch = "riscv64", target_arch = "aarch64", target_arch = "x86_64")))]
#[no_mangle]
pub unsafe extern "C" fn longjmp(_env: *const JmpBuf, val: i32) -> ! {
    let _ = val;
    loop {} // wasm: no real longjmp
}
