//! x86_32 boot entry — multiboot1 header + _start.
//!
//! QEMU `-kernel` loads the ELF, scans the first 8 KiB for the multiboot
//! magic, and enters `_start` in 32-bit protected mode (paging OFF, CS=flat).
//! EAX = 0x2BADB002 (magic), EBX = multiboot_info ptr (unused — FALLBACK_BOOT_INFO).

use core::arch::global_asm;

global_asm!(
    // ── Multiboot 1 header — MUST appear in the first 8 KiB of the binary ─────
    ".section .multiboot, \"a\"",
    ".align 4",
    ".long 0x1BADB002",     // magic
    ".long 0x00000002",     // flags: bit 1 = memory-map requested
    ".long 0xE4524FFC",     // checksum = -(magic + flags) mod 2^32

    // ── Boot entry ─────────────────────────────────────────────────────────────
    ".section .text.boot, \"ax\"",
    ".global _start",
    "_start:",
    "cli",

    // Setup kernel stack (absolute 32-bit address, paging off).
    // `lea edi, [label]` in Intel/x86_32 = load effective address of label.
    "lea esp, [{stack_top}]",
    "and esp, -16",
    "xor ebp, ebp",

    // Clear BSS (zeroed before any Rust code runs).
    "lea edi, [{bss_start}]",
    "lea ecx, [{bss_end}]",
    "sub ecx, edi",         // byte count
    "shr ecx, 2",           // dword count
    "xor eax, eax",
    "rep stosd",

    // Call entry wrapper (no args; wrapper sets hartid=0, dtb=0).
    "call {entry}",
    ".L_halt:",
    "hlt",
    "jmp .L_halt",

    // ── Thread trampoline stub (linker symbol; never called in nano) ───────────
    ".global thread_trampoline",
    "thread_trampoline:",
    ".L_trap_halt:",
    "hlt",
    "jmp .L_trap_halt",

    stack_top = sym __stack_top,
    bss_start = sym __bss_start,
    bss_end   = sym __bss_end,
    entry     = sym kmain_x86_32,
);

/// Bridge from 32-bit `_start` to the architecture-neutral `kmain`.
///
/// x86_32 QEMU brings up QEMU in bare mode; no multiboot info parsed.
/// The kernel uses `FALLBACK_BOOT_INFO` instead.
#[no_mangle]
pub extern "C" fn kmain_x86_32() -> ! {
    extern "C" { fn kmain(hartid: usize, dtb: usize) -> !; }
    // SAFETY: kmain is the kernel C entry point; 0,0 are valid placeholder values.
    unsafe { kmain(0, 0) }
}

// ── Linker-script symbols ────────────────────────────────────────────────────
extern "C" {
    pub static __stack_top: u8;
    pub static mut __bss_start: u8;
    pub static __bss_end: u8;
}
