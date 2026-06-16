//! 32-bit Global Descriptor Table — null + kernel-code (CS=0x08) + kernel-data (DS=0x10).
//!
//! Nano profile: loads the GDT and reloads data segments.
//! CS reload uses a far-jump via the indirect call gate trick.

/// GDT entry for a flat 32-bit segment:
///   `0x00CF_XX00_0000_FFFF` — G=1 (4 KB), DB=1 (32-bit), limit=0xFFFFF → 4 GB.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct GdtEntry(u64);

static mut GDT: [GdtEntry; 3] = [
    GdtEntry(0),                     // 0x00: null
    GdtEntry(0x00CF_9A00_0000_FFFF), // 0x08: kernel code (exec/read, DPL=0, 32-bit)
    GdtEntry(0x00CF_9200_0000_FFFF), // 0x10: kernel data (read/write, DPL=0)
];

#[repr(C, packed)]
struct GdtPointer { limit: u16, base: u32 }

/// Load the 32-bit GDT and reload segment registers.
///
/// CS is reloaded via a push/retf pair. DS/ES/FS/GS/SS are set to selector 0x10.
pub fn init() {
    // SAFETY: GDT is a static; lgdt/retf are required for protected-mode setup.
    unsafe {
        let ptr = GdtPointer {
            limit: (core::mem::size_of_val(&GDT) - 1) as u16,
            base:  GDT.as_ptr() as u32,
        };

        core::arch::asm!(
            // Load GDT register.
            "lgdt [{p}]",
            // Far return to reload CS with kernel-code selector 0x08.
            // Push CS:EIP pair, then lret (32-bit far return).
            "push 0x08",
            "lea eax, [2f]",
            "push eax",
            "retf",           // Intel syntax 32-bit far return
            "2:",
            // Reload data/stack segment registers.
            "mov ax, 0x10",
            "mov ds, ax",
            "mov es, ax",
            "mov fs, ax",
            "mov gs, ax",
            "mov ss, ax",
            p = in(reg) &ptr as *const _ as u32,
            out("eax") _,
            options(nostack),
        );
    }
}
