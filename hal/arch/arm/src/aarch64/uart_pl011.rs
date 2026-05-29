//! ARM PrimeCell PL011 UART driver.
//!
//! QEMU virt machine maps PL011 at 0x09000000.
//! Used for early boot output before the kernel logging system is ready.
//! All register access goes through volatile MMIO writes.

/// PL011 base address on QEMU virt.
pub const PL011_BASE: usize = 0x0900_0000;

// PL011 register offsets (byte offsets from base).
const UARTDR:   usize = 0x000; // Data Register
const UARTFR:   usize = 0x018; // Flag Register
const UARTIBRD: usize = 0x024; // Integer Baud Rate Divisor
const UARTFBRD: usize = 0x028; // Fractional Baud Rate Divisor
const UARTLCR:  usize = 0x02C; // Line Control Register
const UARTCR:   usize = 0x030; // Control Register

const FR_TXFF: u32 = 1 << 5; // TX FIFO full

#[inline(always)]
fn mmio_read(offset: usize) -> u32 {
    // SAFETY: PL011_BASE is a valid identity-mapped MMIO address on QEMU virt.
    unsafe { core::ptr::read_volatile((PL011_BASE + offset) as *const u32) }
}

#[inline(always)]
fn mmio_write(offset: usize, val: u32) {
    // SAFETY: same as mmio_read.
    unsafe { core::ptr::write_volatile((PL011_BASE + offset) as *mut u32, val) }
}

/// Initialise PL011 for 115200 8N1 at 24 MHz reference clock.
///
/// Must be called before any `putchar` call.  Safe to call multiple times.
pub fn init() {
    // Disable UART.
    mmio_write(UARTCR, 0);
    // 24 MHz / (16 × 115200) ≈ 13.02 → IBRD=13, FBRD=1.
    mmio_write(UARTIBRD, 13);
    mmio_write(UARTFBRD, 1);
    // 8-bit, no parity, 1 stop bit, FIFO enabled.
    mmio_write(UARTLCR, (3 << 5) | (1 << 4));
    // Enable UART, TX, RX.
    mmio_write(UARTCR, (1 << 0) | (1 << 8) | (1 << 9));
}

/// Write one byte to the UART, blocking until the TX FIFO has space.
pub fn putchar(byte: u8) {
    while mmio_read(UARTFR) & FR_TXFF != 0 {
        core::hint::spin_loop();
    }
    mmio_write(UARTDR, byte as u32);
}

/// Write a string to the UART, converting `\n` to `\r\n`.
pub fn puts(s: &str) {
    for b in s.bytes() {
        if b == b'\n' {
            putchar(b'\r');
        }
        putchar(b);
    }
}
