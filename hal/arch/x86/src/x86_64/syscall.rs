//! x86_64 SYSCALL/SYSRET MSR configuration.
//! EFER.SCE=1, STAR (segment selectors), LSTAR (entry point), FMASK.
use core::arch::asm;

const IA32_EFER:        u32 = 0xC000_0080;
const IA32_STAR:        u32 = 0xC000_0081;
const IA32_LSTAR:       u32 = 0xC000_0082;
const IA32_FMASK:       u32 = 0xC000_0084;
const IA32_KERNEL_GSBASE: u32 = 0xC000_0102; // Swapped into GS_BASE by swapgs

/// Per-CPU storage used by the `swapgs`-based stack swap in syscall_entry.
///
/// Layout: [0] = kernel RSP (loaded on syscall entry),
///         [8] = scratch (user RSP saved here during syscall).
///
/// KERNEL_GS_BASE MSR must point here before any Ring-3 entry.
/// `set_cpu_local` initialises this; `set_kernel_stack` updates slot [0].
#[repr(C, align(16))]
struct CpuLocal {
    kernel_rsp: u64,
    user_rsp:   u64,
}
static mut CPU_LOCAL: CpuLocal = CpuLocal { kernel_rsp: 0, user_rsp: 0 };

fn rdmsr(msr: u32) -> u64 {
    let lo:u32; let hi:u32;
    // SAFETY: rdmsr from Ring 0 does not affect memory safety.
    unsafe { asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi, options(nomem,nostack)); }
    (hi as u64)<<32 | lo as u64
}
fn wrmsr(msr: u32, val: u64) {
    let lo=val as u32; let hi=(val>>32) as u32;
    // SAFETY: wrmsr to a valid MSR from Ring 0 does not affect memory safety.
    unsafe { asm!("wrmsr", in("ecx") msr, in("eax") lo, in("edx") hi, options(nomem,nostack)); }
}

/// Initialise SYSCALL/SYSRET path and per-CPU GS area.
///
/// Must be called from Ring 0 before any Ring-3 entry.  Sets up:
/// - EFER.SCE so the CPU honours the SYSCALL instruction
/// - STAR/LSTAR/FMASK for the entry point and segment selectors
/// - KERNEL_GS_BASE pointing at `CPU_LOCAL` so `swapgs` in the syscall
///   entry stub can load the kernel stack without touching user memory
pub fn init() {
    wrmsr(IA32_EFER, rdmsr(IA32_EFER)|1); // SCE=1
    // STAR: user CS=0x20 (sysret CS=0x23, SS=0x2B=uDS),
    //       kernel CS=0x08 (syscall CS=0x08, SS=0x10=kDS)
    wrmsr(IA32_STAR, (0x0020_u64<<48)|(0x0008_u64<<32));
    extern "C" { fn syscall_entry(); }
    wrmsr(IA32_LSTAR, syscall_entry as *const () as u64);
    wrmsr(IA32_FMASK, 0x0300); // clear IF + DF on syscall entry

    // Point KERNEL_GS_BASE at the per-CPU area so swapgs in syscall_entry
    // exchanges GS_BASE with KERNEL_GS_BASE and gives us %gs:0 / %gs:8.
    // SAFETY: CPU_LOCAL is a static; addr_of! gives a raw pointer without
    // creating a Rust reference.
    // addr_of! on a static does not require unsafe (no Rust reference created).
    let cpu_local_addr = core::ptr::addr_of!(CPU_LOCAL) as u64;
    wrmsr(IA32_KERNEL_GSBASE, cpu_local_addr);
}

/// Update the kernel-stack pointer stored in the per-CPU area.
///
/// Called by the scheduler before every Ring-3 entry so that `swapgs` +
/// `movq %gs:0, %rsp` in `syscall_entry` loads the correct kernel stack.
pub fn set_kernel_stack(sp: u64) {
    // SAFETY: CPU_LOCAL is a static with no aliased Rust references here.
    unsafe { CPU_LOCAL.kernel_rsp = sp; }
}

/// Placeholder Rust syscall dispatcher (called from syscall_entry asm).
#[no_mangle]
pub extern "C" fn x86_64_syscall_dispatch() {
    // TODO: forward to the kernel syscall table.
}

use core::arch::global_asm;

// The syscall_entry stub uses AT&T syntax (global_asm default on x86_64).
// On SYSCALL entry: RCX = user RIP, R11 = user RFLAGS, RSP = user RSP.
global_asm!(r#"
    .section .text
    .global syscall_entry
    .balign 16
syscall_entry:
    swapgs
    # save user RSP; load kernel RSP from per-CPU area via GS
    movq %rsp,  %gs:8
    movq %gs:0, %rsp
    # push syscall frame
    pushq %rcx       # user RIP
    pushq %r11       # user RFLAGS
    pushq %rdi
    pushq %rsi
    pushq %rdx
    pushq %r10
    pushq %r8
    pushq %r9
    call  x86_64_syscall_dispatch
    popq  %r9
    popq  %r8
    popq  %r10
    popq  %rdx
    popq  %rsi
    popq  %rdi
    popq  %r11
    popq  %rcx
    movq  %gs:8, %rsp
    swapgs
    sysretq
"#);
