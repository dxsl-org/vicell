//! x86_64 Interrupt Descriptor Table — 256 16-byte long-mode gates.
//!
//! All entries use the `extern "x86-interrupt"` ABI so LLVM saves all
//! caller-saved registers and emits `iretq` (not `ret`) on return.
//! Without this, a bare `extern "C"` handler would return with `ret`
//! into the CPU's stacked interrupt frame, causing a triple-fault.

use core::arch::asm;

/// Interrupt stack frame pushed by the CPU on every exception/interrupt entry.
#[repr(C)]
pub struct InterruptFrame {
    pub rip:    u64,
    pub cs:     u64,
    pub rflags: u64,
    pub rsp:    u64,
    pub ss:     u64,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct IdtEntry {
    off_lo: u16, sel: u16, ist: u8, attr: u8, off_mid: u16, off_hi: u32, _res: u32,
}
impl IdtEntry {
    fn new(handler: u64, dpl: u8) -> Self {
        Self {
            off_lo:  (handler & 0xFFFF) as u16,
            sel:     0x08,
            ist:     0,
            attr:    0x8E | ((dpl & 3) << 5),
            off_mid: ((handler >> 16) & 0xFFFF) as u16,
            off_hi:  ((handler >> 32) & 0xFFFF_FFFF) as u32,
            _res:    0,
        }
    }
}

#[repr(C, align(16))]
struct Idt { e: [IdtEntry; 256] }
#[repr(C, packed)]
struct IdtPtr { limit: u16, base: u64 }

static mut IDT: Idt = Idt { e: [IdtEntry { off_lo:0, sel:0, ist:0, attr:0, off_mid:0, off_hi:0, _res:0 }; 256] };

pub fn init() {
    // SAFETY: single-threaded boot; IDT is a static global.
    unsafe {
        // SAFETY: addr_of_mut! avoids creating a Rust reference to a mutable static.
        let idt_ptr = core::ptr::addr_of_mut!(IDT);

        // General handler for most vectors (no CPU-pushed error code).
        let handler_addr = x86_64_irq_handler as *const () as u64;
        for e in (*idt_ptr).e.iter_mut() { *e = IdtEntry::new(handler_addr, 0); }

        // Exceptions that push an error code: #DF=8, #TS=10, #NP=11, #SS=12, #GP=13, #PF=14, #AC=17
        let ec_handler = x86_64_ec_handler as *const () as u64;
        for vec in [8u8, 10, 11, 12, 13, 14, 17] {
            (*idt_ptr).e[vec as usize] = IdtEntry::new(ec_handler, 0);
        }

        // Vector 0x80: DPL=3 so user code can issue `int 0x80` (legacy, tolerated).
        (*idt_ptr).e[0x80] = IdtEntry::new(handler_addr, 3);

        let ptr = IdtPtr {
            limit: (core::mem::size_of::<Idt>()-1) as u16,
            base: core::ptr::addr_of!((*idt_ptr).e) as u64,
        };
        // SAFETY: ptr points to a valid, aligned IDT; lidt from Ring 0 is safe.
        asm!("lidt [{p}]", p = in(reg) &ptr, options(nomem, nostack));
    }
}

/// Handler for IRQs and exceptions WITHOUT a CPU-pushed error code.
///
/// `extern "x86-interrupt"` causes LLVM to save all registers and return
/// via `iretq`, matching the CPU's expectation on interrupt return.
#[no_mangle]
extern "x86-interrupt" fn x86_64_irq_handler(frame: InterruptFrame) {
    // Dispatch based on vector.  Without per-vector stubs we do not know
    // the vector number here; LAPIC timer is the primary expected IRQ.
    // TODO: generate per-vector stubs that push the vector index.
    super::apic::eoi();
    let _ = frame;
}

/// Handler for exceptions WITH a CPU-pushed error code (#GP, #PF, etc.).
#[no_mangle]
extern "x86-interrupt" fn x86_64_ec_handler(frame: InterruptFrame, error_code: u64) {
    let cr2: u64;
    // SAFETY: reading CR2 does not modify any state.
    unsafe { asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack)); }
    panic!(
        "[x86_64] exception rip=0x{:X} ec=0x{:X} cr2=0x{:X}",
        frame.rip, error_code, cr2
    );
}
