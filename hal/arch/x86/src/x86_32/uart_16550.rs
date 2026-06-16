//! COM1 (16550A UART) driver for x86_32 — port I/O at 0x3F8.
//!
//! Identical behaviour to the x86_64 version; duplicated here because
//! the `target_arch = "x86"` build has its own HAL export chain.

const COM1: u16 = 0x3F8;

fn outb(port: u16, val: u8) {
    // SAFETY: writing to a well-known UART port; no memory aliasing.
    unsafe { core::arch::asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack)); }
}

fn inb(port: u16) -> u8 {
    let val: u8;
    // SAFETY: reading from a well-known UART port; no memory aliasing.
    unsafe { core::arch::asm!("in al, dx", in("dx") port, out("al") val, options(nomem, nostack)); }
    val
}

/// Initialise COM1 at 115200 baud, 8N1.
pub fn init() {
    outb(COM1 + 1, 0x00); // disable interrupts
    outb(COM1 + 3, 0x80); // set DLAB
    outb(COM1 + 0, 0x01); // divisor low  (115200 baud @ 1.8432 MHz)
    outb(COM1 + 1, 0x00); // divisor high
    outb(COM1 + 3, 0x03); // 8N1, clear DLAB
    outb(COM1 + 2, 0xC7); // FIFO control: enable, clear, 14-byte threshold
    outb(COM1 + 4, 0x0B); // modem control: DTR, RTS, out2
}

/// Write a single byte to COM1, blocking until the transmit FIFO is ready.
pub fn putchar(byte: u8) {
    while inb(COM1 + 5) & 0x20 == 0 {
        core::hint::spin_loop();
    }
    outb(COM1, byte);
}
