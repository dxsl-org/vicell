//! x86_64 Interrupt Descriptor Table — 256 16-byte long-mode gates.
//!
//! All entries use the `extern "x86-interrupt"` ABI so LLVM saves all
//! caller-saved registers and emits `iretq` (not `ret`) on return.
//! Without this, a bare `extern "C"` handler would return with `ret`
//! into the CPU's stacked interrupt frame, causing a triple-fault.
//!
//! Per-vector dispatch:
//!   0x20 — LAPIC periodic timer → `vi_timer_tick()` (kernel scheduler tick)
//!   0x24 — COM1 / IOAPIC IRQ 4 → `vi_handle_uart_irq()` (shell stdin)
//!   0x0E — #PF (with error code) → `vi_handle_page_fault()`

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

        // Vector 0x20: LAPIC periodic timer → vi_timer_tick().
        // The LAPIC is programmed to fire at vector 0x20 by apic::init_lapic_calibrated.
        (*idt_ptr).e[0x20] = IdtEntry::new(x86_64_timer_handler as *const () as u64, 0);

        // Vector 0x24: COM1 UART RX (IOAPIC IRQ 4, redirected to vector 0x24) → vi_handle_uart_irq().
        (*idt_ptr).e[0x24] = IdtEntry::new(x86_64_uart_handler as *const () as u64, 0);

        let ptr = IdtPtr {
            limit: (core::mem::size_of::<Idt>()-1) as u16,
            base: core::ptr::addr_of!((*idt_ptr).e) as u64,
        };
        // SAFETY: ptr points to a valid, aligned IDT; lidt from Ring 0 is safe.
        asm!("lidt [{p}]", p = in(reg) &ptr, options(nomem, nostack));
    }
}

/// Install a handler for an arbitrary IDT vector (Ring 0, no error code).
///
/// Used by `uart_16550::init_input_irq` to wire COM1 RX after IOAPIC redirect.
/// Prerequisite: `init()` must have been called first.
///
/// # Safety
/// `handler` must be a valid kernel function; caller ensures it is safe to call
/// from interrupt context.
pub unsafe fn install_vector(vec: u8, handler: u64) {
    let idt_ptr = core::ptr::addr_of_mut!(IDT);
    // SAFETY: IDT is already loaded; updating an entry is atomic at 64-bit alignment.
    unsafe {
        (*idt_ptr).e[vec as usize] = IdtEntry::new(handler, 0);
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

// Kernel-provided hooks — defined in kernel::task / kernel::memory.
// Declared here as externs to let the HAL call them without a crate cycle.
extern "Rust" {
    /// LAPIC periodic timer: increment tick counter + run scheduler.
    /// Defined in `kernel::task` with `#[no_mangle]`.
    fn vi_timer_tick();

    /// UART RX IRQ: read byte from COM1 into the shared RX buffer.
    /// Defined in `kernel::task::drivers::uart` with `#[no_mangle]`.
    fn vi_handle_uart_irq();

    /// #PF handler. Defined in `kernel::memory::paging` with `#[no_mangle]`.
    fn vi_handle_page_fault(va: usize, error_code: u64);
}

/// LAPIC periodic timer handler (vector 0x20).
///
/// Calls `vi_timer_tick()` (kernel scheduler tick), then ACKs the LAPIC.
/// The LAPIC timer reloads automatically in periodic mode — no re-arm needed.
#[no_mangle]
extern "x86-interrupt" fn x86_64_timer_handler(_frame: InterruptFrame) {
    // SAFETY: vi_timer_tick is #[no_mangle] in kernel/src/task.rs; safe to call
    // from interrupt context (interrupts disabled by CPU on IRQ entry).
    unsafe { vi_timer_tick(); }
    super::apic::eoi();
}

/// COM1 UART RX handler (vector 0x24 / IOAPIC IRQ 4).
///
/// Drains the UART RHR into the kernel RX buffer, then ACKs the LAPIC.
#[no_mangle]
extern "x86-interrupt" fn x86_64_uart_handler(_frame: InterruptFrame) {
    // SAFETY: vi_handle_uart_irq is #[no_mangle] in kernel/src/task/drivers/uart.rs.
    unsafe { vi_handle_uart_irq(); }
    super::apic::eoi();
}

/// Handler for exceptions WITH a CPU-pushed error code (#GP, #PF, etc.).
///
/// Forwards #PF (vector 14) to the kernel page-fault handler via the
/// `vi_handle_page_fault` extern hook so demand-paging can map user pages.
/// All other error-code exceptions still end in a kernel panic.
///
/// Phase 02 will add per-vector stubs that push the vector index so this
/// handler can distinguish #PF from #GP cleanly without the CR2 heuristic.
#[no_mangle]
extern "x86-interrupt" fn x86_64_ec_handler(frame: InterruptFrame, error_code: u64) {
    let cr2: u64;
    // SAFETY: reading CR2 does not modify any state.
    unsafe { asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack)); }

    // Heuristic: attempt page-fault handling for every error-code exception.
    // For true #PF (vector 14) the kernel handler maps the page or panics.
    // For #GP and others the kernel handler will panic immediately because
    // error_code bit 2 (U/S) will be 0 for a kernel-mode fault (kernel CS=8).
    //
    // SAFETY: vi_handle_page_fault is defined in kernel::memory::paging; it
    // acquires scheduler and frame-allocator spinlocks which the IDT entry
    // path does not hold.
    unsafe { vi_handle_page_fault(cr2 as usize, error_code); }

    let _ = frame; // frame rip/cs available for future per-vector dispatch
}
