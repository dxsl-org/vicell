//! 32-bit Interrupt Descriptor Table — 256 stub entries pointing to a halt loop.
//!
//! Nano profile: no real interrupt handlers. All vectors redirect to a halt stub.
//! Presence of a valid IDT prevents GP-faults from triple-faulting before
//! the machine is ready to process them.

use core::arch::global_asm;

/// 32-bit interrupt gate descriptor (8 bytes).
/// Format: [off_lo | sel | 0x00 | attr | off_hi]
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_lo: u16,
    selector:  u16,
    zero:      u8,
    /// 0x8E = present | DPL=0 | 32-bit interrupt gate.
    attr:      u8,
    offset_hi: u16,
}

impl IdtEntry {
    const fn new(handler: u32) -> Self {
        IdtEntry {
            offset_lo: handler as u16,
            selector:  0x0008,           // kernel code segment
            zero:      0,
            attr:      0x8E,
            offset_hi: (handler >> 16) as u16,
        }
    }

    const fn null() -> Self {
        IdtEntry { offset_lo: 0, selector: 0, zero: 0, attr: 0, offset_hi: 0 }
    }
}

// Halt-loop stub for all interrupt vectors.
global_asm!(
    ".global __isr_stub_x86_32",
    "__isr_stub_x86_32:",
    ".L_isr_halt:",
    "cli",
    "hlt",
    "jmp .L_isr_halt",
);
extern "C" { fn __isr_stub_x86_32(); }

// Build the table after knowing the stub address.
// IDT must be initialized at runtime because we need the handler's runtime address.
#[repr(C, align(8))]
struct Idt([IdtEntry; 256]);

static mut IDT: Idt = Idt([IdtEntry::null(); 256]);

#[repr(C, packed)]
struct IdtPointer { limit: u16, base: u32 }

/// Populate all 256 IDT entries with the halt stub and load `lidt`.
pub fn init() {
    // SAFETY: IDT is a static; address obtained at runtime.
    unsafe {
        let handler = __isr_stub_x86_32 as u32;
        for entry in IDT.0.iter_mut() {
            *entry = IdtEntry::new(handler);
        }
        let ptr = IdtPointer {
            limit: (core::mem::size_of::<Idt>() - 1) as u16,
            base:  IDT.0.as_ptr() as u32,
        };
        core::arch::asm!("lidt [{ptr}]", ptr = in(reg) &ptr, options(nostack, readonly));
    }
}
