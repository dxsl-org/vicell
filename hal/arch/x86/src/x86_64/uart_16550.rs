//! 16550A UART driver via x86 port I/O. COM1 base: 0x3F8.
//!
//! Two-phase init:
//!   1. `init()` — baud-rate, framing, FIFO setup (IRQs disabled; used at early boot).
//!   2. `init_input_irq()` — enable UART RX IRQ + redirect IOAPIC IRQ 4 to IDT vector 0x24.
//!      Call this AFTER the IOAPIC (and LAPIC) are live.

const COM1: u16 = 0x3F8;

/// IOAPIC IRQ number for COM1 (ISA IRQ 4).
const UART_IRQ: u8 = 4;
/// IDT vector allocated for COM1 RX interrupts.
pub const UART_VECTOR: u8 = 0x24;

#[inline]
fn outb(port: u16, val: u8) {
    // SAFETY: port I/O on COM1 does not affect memory safety.
    unsafe { core::arch::asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack)); }
}
#[inline]
fn inb(port: u16) -> u8 {
    let val: u8;
    // SAFETY: reading port I/O does not affect memory safety.
    unsafe { core::arch::asm!("in al, dx", in("dx") port, out("al") val, options(nomem, nostack)); }
    val
}

/// Initialise COM1 at 115200 8N1. IRQs intentionally left DISABLED here;
/// call `init_input_irq()` later to enable them once the IOAPIC/LAPIC are ready.
pub fn init() {
    outb(COM1 + 1, 0x00); // Disable IRQs
    outb(COM1 + 3, 0x80); // DLAB = 1
    outb(COM1 + 0, 0x01); // Divisor low  (115200 baud)
    outb(COM1 + 1, 0x00); // Divisor high
    outb(COM1 + 3, 0x03); // 8N1
    outb(COM1 + 2, 0xC7); // FIFO, 14-byte threshold
    outb(COM1 + 4, 0x0B); // MCR: OUT2 (enables IOAPIC IRQ delivery) + RTS + DSR
}

/// Enable COM1 RX interrupts and route IOAPIC IRQ 4 → IDT vector 0x24.
///
/// Preconditions: `init()` called, LAPIC and IOAPIC are initialised
/// (i.e. after `crate::init_timers()` in kmain).
///
/// After this call, each received byte fires vector 0x24, which calls
/// `vi_handle_uart_irq()` → pushes the byte into the kernel RX buffer →
/// the shell's `sys_recv` on the input service drains it.
pub fn init_input_irq() {
    // 1. Enable UART RX-ready interrupt (IER bit 0).
    outb(COM1 + 1, 0x01);

    // 2. Wire IOAPIC ISA IRQ 4 → IDT vector 0x24 on CPU 0.
    //    ioapic_redirect(irq, vec) sets: destination=CPU 0, edge-triggered, active-high.
    super::apic::ioapic_redirect(UART_IRQ, UART_VECTOR);
}

/// Write one byte, blocking on TX hold register empty.
pub fn putchar(byte: u8) {
    while inb(COM1 + 5) & 0x20 == 0 { core::hint::spin_loop(); }
    outb(COM1, byte);
}
/// Write string, converting `\n` to `\r\n`.
pub fn puts(s: &str) {
    for b in s.bytes() {
        if b == b'\n' { putchar(b'\r'); }
        putchar(b);
    }
}
