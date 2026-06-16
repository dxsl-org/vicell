//! PL011 UART driver for ARM32 — MMIO at 0x09000000 (QEMU virt machine).
//!
//! Register accesses use 32-bit volatile MMIO writes (ARM32 has no 64-bit registers).
//! Base address and baud settings are identical to the AArch64 version.

const PL011_BASE: u32 = 0x0900_0000; // QEMU ARM virt machine PL011

const UARTDR:   u32 = PL011_BASE;          // Data Register (TX/RX)
const UARTFR:   u32 = PL011_BASE + 0x018;  // Flag Register
const UARTIBRD: u32 = PL011_BASE + 0x024;  // Integer Baud Rate Divisor
const UARTFBRD: u32 = PL011_BASE + 0x028;  // Fractional Baud Rate Divisor
const UARTLCR:  u32 = PL011_BASE + 0x02C;  // Line Control Register
const UARTCR:   u32 = PL011_BASE + 0x030;  // Control Register

#[inline(always)]
unsafe fn mmio_write(addr: u32, val: u32) {
    // SAFETY: caller guarantees addr is a valid PL011 MMIO register.
    core::ptr::write_volatile(addr as *mut u32, val);
}

#[inline(always)]
unsafe fn mmio_read(addr: u32) -> u32 {
    // SAFETY: caller guarantees addr is a valid PL011 MMIO register.
    core::ptr::read_volatile(addr as *const u32)
}

/// Initialise PL011 at 115200 baud (48 MHz reference clock), 8N1, FIFO enabled.
pub fn init() {
    // SAFETY: MMIO addresses are fixed QEMU virt device addresses.
    unsafe {
        mmio_write(UARTCR,   0);        // disable UART
        mmio_write(UARTIBRD, 26);       // integer divisor  (115200 baud @ 48 MHz)
        mmio_write(UARTFBRD, 3);        // fractional divisor
        mmio_write(UARTLCR,  0x70);    // 8N1 + FIFO enable (FEN bit)
        mmio_write(UARTCR,   0x301);   // TXE | RXE | UARTEN
    }
}

/// Write a single byte, blocking until the TX FIFO is not full.
pub fn putchar(c: u8) {
    // SAFETY: MMIO addresses are valid PL011 registers; blocking spin is safe.
    unsafe {
        while mmio_read(UARTFR) & (1 << 5) != 0 {
            core::hint::spin_loop();
        }
        mmio_write(UARTDR, c as u32);
    }
}
