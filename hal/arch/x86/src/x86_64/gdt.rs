//! x86_64 GDT + TSS. Selectors: null=0 kCS=0x08 kDS=0x10 uDS=0x18 uCS=0x20 TSS=0x28.
use core::arch::asm;

/// Minimal TSS storing only the kernel-stack pointer (RSP0).
#[repr(C, packed)]
pub struct Tss {
    _r0: u32,
    pub rsp0: u64,
    _rest: [u8; 84],
}
impl Tss { pub const fn new() -> Self { Self { _r0: 0, rsp0: 0, _rest: [0; 84] } } }

#[repr(transparent)]
#[derive(Copy, Clone)]
struct GdtEntry(u64);
impl GdtEntry {
    const NULL: Self = Self(0);
    const fn code(dpl: u8) -> Self {
        Self((1u64<<43)|(1<<44)|((dpl as u64)<<45)|(1<<47)|(1<<53))
    }
    const fn data(dpl: u8) -> Self {
        Self((1u64<<41)|(1<<44)|((dpl as u64)<<45)|(1<<47))
    }
    fn tss_low(base: u64, limit: u32) -> Self {
        let b = ((base&0xFF)<<16)|((base>>8&0xFF)<<24)|((base>>16&0xFF)<<32)|((base>>24&0xFF)<<56);
        let l = (limit as u64 & 0xFFFF) | ((limit as u64 >>16)<<48);
        Self(l|b|(0x9u64<<40)|(1<<47))
    }
    fn tss_high(base: u64) -> Self { Self((base>>32)&0xFFFF_FFFF) }
}

#[repr(C, align(16))]
struct Gdt { entries: [GdtEntry; 8] }
#[repr(C, packed)]
struct GdtPtr { limit: u16, base: u64 }

static mut GDT: Gdt = Gdt { entries: [GdtEntry::NULL; 8] };
pub static mut TSS: Tss = Tss::new();

/// Build and install the GDT + TSS.
pub fn init() {
    // SAFETY: single-threaded boot; GDT and TSS are static globals.
    unsafe {
        GDT.entries[1] = GdtEntry::code(0);
        GDT.entries[2] = GdtEntry::data(0);
        GDT.entries[3] = GdtEntry::data(3);
        GDT.entries[4] = GdtEntry::code(3);
        // SAFETY: addr_of_mut!/addr_of! avoids creating a Rust reference to a mutable static.
        let b = core::ptr::addr_of!(TSS) as u64;
        let l = (core::mem::size_of::<Tss>()-1) as u32;
        GDT.entries[5] = GdtEntry::tss_low(b, l);
        GDT.entries[6] = GdtEntry::tss_high(b);
        let ptr = GdtPtr {
            limit: (core::mem::size_of::<Gdt>()-1) as u16,
            base: core::ptr::addr_of!(GDT) as u64,
        };
        asm!(
            // SAFETY: GDT pointer is valid; lgdt + far jmp reload CS; ltr loads TSS.
            "lgdt [{p}]",
            "push 0x08",
            "lea {t}, [rip+99f]",
            "push {t}",
            "retfq",
            "99:",
            "mov ax, 0x10",
            "mov ds, ax", "mov es, ax", "mov ss, ax",
            "mov ax, 0x28", "ltr ax",
            p = in(reg) &ptr, t = lateout(reg) _, // Intel syntax
        );
    }
}
/// Set RSP0 (kernel stack for Ring3->Ring0 transition).
pub fn set_kernel_stack(sp: u64) {
    // SAFETY: TSS is static; single-threaded spawn path.
    unsafe { TSS.rsp0 = sp; }
}
