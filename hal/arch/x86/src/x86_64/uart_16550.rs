//! 16550A UART driver via x86 port I/O. COM1 base: 0x3F8.
const COM1: u16 = 0x3F8;
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
/// Initialise COM1 at 115200 8N1.
pub fn init() {
    outb(COM1 + 1, 0x00); // Disable IRQs
    outb(COM1 + 3, 0x80); // DLAB = 1
    outb(COM1 + 0, 0x01); // Divisor low  (115200 baud)
    outb(COM1 + 1, 0x00); // Divisor high
    outb(COM1 + 3, 0x03); // 8N1
    outb(COM1 + 2, 0xC7); // FIFO, 14-byte threshold
    outb(COM1 + 4, 0x0B); // IRQs enabled, RTS/DSR
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
