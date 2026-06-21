use crate::syscall::sys_log;
use core::panic::PanicInfo;

#[no_mangle]
#[unsafe(naked)]
#[link_section = ".text.boot"]
pub unsafe extern "C" fn _start() -> ! {
    #[cfg(target_arch = "riscv64")]
    core::arch::naked_asm!(
        ".option push",
        ".option norelax",
        "la gp, __global_pointer$",
        ".option pop",
        "andi sp, sp, -16",
        // Use callee-saved s0/s1 as iterators — a0/a1 are caller-saved and
        // would be clobbered by any constructor called via jalr.
        "la   s0, __init_array_start",
        "la   s1, __init_array_end",
        "1:",
        "beq  s0, s1, 2f",
        "ld   t0, 0(s0)",
        "jalr t0",
        "addi s0, s0, 8",
        "j    1b",
        "2:",
        "call main",
        "li a7, 60",   // ViSyscall::Exit
        "li a0, 0",    // exit code = 0 in a0 (ViCell ABI: syscall nr in a7, arg in a0)
        "ecall",
        "1: j 1b"
    );
    // ViCell ARM64 ABI: x0=syscall_nr, x1=a0 (exit code).
    // Stack is kernel-aligned on entry; skip re-alignment to avoid clobbering sp.
    #[cfg(target_arch = "aarch64")]
    core::arch::naked_asm!(
        "ldr  x19, =__init_array_start",
        "ldr  x20, =__init_array_end",
        "1:",
        "cmp  x19, x20",
        "b.eq 2f",
        "ldr  x21, [x19], #8",
        "blr  x21",
        "b    1b",
        "2:",
        "bl   main",
        "mov  x0, #60",   // ViSyscall::Exit
        "mov  x1, #0",    // exit code = 0
        "svc  #0",
        "1: b 1b"
    );
    // ViCell x86_64 ABI: RAX=syscall_nr, RDI=a0.
    // The kernel-supplied stack is 16-byte-aligned at entry; AND to guarantee it
    // before the CALL so the SysV AMD64 ABI requirement is always satisfied.
    // sys_exit(0): ViSyscall::Exit = 60, exit code in RDI = 0.
    #[cfg(target_arch = "x86_64")]
    core::arch::naked_asm!(
        "and rsp, -16",
        "lea rbx, [rip + __init_array_start]",
        "lea r12, [rip + __init_array_end]",
        "1:",
        "cmp rbx, r12",
        "je 2f",
        "call [rbx]",
        "add rbx, 8",
        "jmp 1b",
        "2:",
        "call main",
        "mov rax, 60",    // ViSyscall::Exit
        "xor rdi, rdi",   // exit code = 0
        "syscall",
        "2: jmp 2b",
    );
}

// User applications must define `fn main() -> !` or `fn main()`.
// Since we don't have a standardized `main` signature yet in ostd macro,
// we will assume the app defines `no_mangle pub extern "C" fn main()`.
extern "C" {
    fn main();
}

#[no_mangle]
pub extern "C" fn generic_main() -> ! {
    unsafe {
        main();
    }
    // If main returns, we exit
    crate::syscall::sys_exit(0);
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // Log panic
    // We don't have a proper writer yet, so just stringify manually?
    // Or just sys_log("PANIC!");
    let _ = sys_log("PANIC: Application crashed!\n");
    if let Some(location) = info.location() {
        // simple formatting
        let _ = sys_log("Location: ");
        let _ = sys_log(location.file());
    }

    // Exit
    crate::syscall::sys_exit(1);
}
