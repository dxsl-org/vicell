//! Minimal 16550 UART Driver for QEMU RISC-V Virt
//!
//! Used for kernel logging and early debug output.
//! Base Address: 0x10000000

use crate::sync::Spinlock;
use core::fmt;

/// UART Registers (offset from base)
const _RHR: usize = 0; // Receive Holding Register (read)
const _THR: usize = 0; // Transmit Holding Register (write)
const IER: usize = 1; // Interrupt Enable Register
const FCR: usize = 2; // FIFO Control Register
const _ISR: usize = 2; // Interrupt Status Register
const LCR: usize = 3; // Line Control Register
const LSR: usize = 5; // Line Status Register

/// Line Status Flags
const _LSR_RX_READY: u8 = 1 << 0;
const _LSR_TX_EMPTY: u8 = 1 << 5;

#[allow(non_camel_case_types)]
pub struct viUART {
    base_addr: usize,
}

impl viUART {
    /// Create a new viUART instance (unsafe because base_addr must be valid)
    pub const unsafe fn new(base_addr: usize) -> Self {
        Self { base_addr }
    }

    /// Update the MMIO base address. Called once by `uart::init` from DTB info.
    pub fn set_base(&mut self, base: usize) {
        self.base_addr = base;
    }

    /// Initialize the UART
    pub fn init(&mut self) {
        unsafe {
            let ptr = self.base_addr as *mut u8;

            // Disable interrupts
            ptr.add(IER).write_volatile(0x00);

            // Enable + clear FIFO (bit0=enable, bit1=clear RX, bit2=clear TX).
            ptr.add(FCR).write_volatile(0x07);

            // Set 8-bit mode (Word Length Select bits 0 and 1)
            ptr.add(LCR).write_volatile(0x03);

            // Keep UART RX interrupts DISABLED (IER=0): the console driver polls
            // the RHR directly. If RX IRQs were enabled, OpenSBI's M-mode console
            // handler could drain the RHR before the kernel's S-mode poll sees
            // the byte, swallowing all keyboard input.
            ptr.add(IER).write_volatile(0x00);
        }
    }

    // /// Write a single byte (Unused - Output via SBI)
    // pub fn write_byte(&mut self, byte: u8) { ... }
}

// impl fmt::Write for viUART { ... }

// Global Serial Instance protected by Spinlock
pub static SERIAL: Spinlock<viUART> = Spinlock::new(unsafe { viUART::new(0x10_000_000) });

// Direct writer to avoid stack buffering issues
struct DirectWriter;

impl fmt::Write for DirectWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.bytes() {
            #[cfg(target_arch = "riscv64")]
            { let _ = crate::hal::sbi::console_putchar(c); }
            #[cfg(target_arch = "aarch64")]
            { crate::hal::uart_pl011::putchar(c); }
            #[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
            { crate::hal::uart_16550::putchar(c); }
        }
        Ok(())
    }
}

/// Write a string straight to the console UART, bypassing the `log` level filter.
///
/// USER stdout (cell `println`/`sys_log`) MUST always appear regardless of the
/// kernel's `log::max_level` — it is application output, not kernel debug chatter.
/// Routing it through `log::info!` (as `print_user_log` once did) meant lowering
/// the kernel log level to silence boot spam also silenced the shell prompt.
pub fn write_console(s: &str) {
    use fmt::Write;
    let _ = DirectWriter.write_str(s);
}

// Logger integration
struct SimpleLogger;

impl log::Log for SimpleLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            use fmt::Write;
            let mut writer = DirectWriter;
            let _ = write!(writer, "[{:>5}] {}\n", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

static LOGGER: SimpleLogger = SimpleLogger;

pub fn init() {
    #[cfg(target_arch = "riscv64")]
    {
        // Read UART base from DTB (platform::init must have run first).
        let base = crate::platform::with(|p| p.uart_base);
        SERIAL.lock().set_base(base);
        SERIAL.lock().init();
    }
    // Register the log backend (works on all architectures; DirectWriter routes
    // to the correct UART per target_arch inside write_str).
    let _ = log::set_logger(&LOGGER).map(|()| log::set_max_level(log::LevelFilter::Info));
    // Logged here (not in platform::init) because the logger only just came up —
    // platform::init's own log line is emitted before set_logger and is lost.
    #[cfg(target_arch = "riscv64")]
    log::info!("[uart] RX/TX base = {:#x}", SERIAL.lock().base_addr);
}

// --- Input Handling ---

use alloc::collections::VecDeque;

// Global RX Buffer (Initialized late)
pub static RX_BUFFER: Spinlock<Option<VecDeque<u8>>> = Spinlock::new(None);

/// Initialize Input Buffer (Must be called after Heap Init)
pub fn init_input() {
    *RX_BUFFER.lock() = Some(VecDeque::with_capacity(128));
    log::info!("UART Input Buffer Initialized");
}

/// Poll for a character from the IRQ-filled buffer.
pub fn getchar() -> Option<u8> {
    if let Some(buf) = RX_BUFFER.lock().as_mut() {
        return buf.pop_front();
    }
    None
}

/// Directly poll the 16550 Receive Holding Register.
///
/// This is the most robust input path on QEMU virt: it does not depend on
/// PLIC interrupt delegation to S-mode (which OpenSBI may keep in M-mode) nor
/// on the SBI DBCN console-read extension being implemented. Returns the byte
/// if LSR.DR (Data Ready, bit 0) is set, else `None`.
pub fn poll_rhr() -> Option<u8> {
    let serial = SERIAL.lock();
    // SAFETY: base_addr is the identity-mapped QEMU virt UART MMIO region,
    // mapped explicitly in init_kernel_paging. RHR (offset 0) is read-only and
    // side-effect-free to read when LSR.DR is clear, but we gate on DR anyway.
    unsafe {
        let ptr = serial.base_addr as *mut u8;
        if (ptr.add(LSR).read_volatile() & _LSR_RX_READY) != 0 {
            Some(ptr.add(_RHR).read_volatile())
        } else {
            None
        }
    }
}

/// Called from the UART RX IRQ handler.
///
/// On RISC-V / AArch64: reads from the MMIO base stored in SERIAL.
/// On x86_64: reads directly from COM1 port I/O (port 0x3F8).
/// Handles CR→LF normalisation and pushes bytes into RX_BUFFER for the shell.
#[no_mangle]
pub extern "Rust" fn vi_handle_uart_irq() {
    if let Some(buf) = RX_BUFFER.lock().as_mut() {
        // Drain the UART receive FIFO; stop when no more data is ready.
        loop {
            // Read LSR (offset 5) to check Data Ready (bit 0).
            let (lsr, rhr_byte): (u8, Option<u8>) = {
                #[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
                {
                    // Port I/O path for x86.
                    let lsr = unsafe {
                        let v: u8;
                        core::arch::asm!(
                            "in al, dx",
                            in("dx") (0x3F8u16 + LSR as u16),
                            out("al") v,
                            options(nomem, nostack)
                        );
                        v
                    };
                    let byte = if lsr & (_LSR_RX_READY as u8) != 0 {
                        let c: u8;
                        unsafe {
                            core::arch::asm!(
                                "in al, dx",
                                in("dx") 0x3F8u16,
                                out("al") c,
                                options(nomem, nostack)
                            );
                        }
                        Some(c)
                    } else {
                        None
                    };
                    (lsr, byte)
                }
                #[cfg(not(any(target_arch = "x86_64", target_arch = "x86")))]
                {
                    // MMIO path for RISC-V / AArch64.
                    let serial = SERIAL.lock();
                    let ptr = serial.base_addr as *mut u8;
                    // SAFETY: MMIO region is identity-mapped and valid.
                    let lsr_val = unsafe { ptr.add(LSR).read_volatile() };
                    let byte = if lsr_val & (_LSR_RX_READY as u8) != 0 {
                        Some(unsafe { ptr.add(_RHR).read_volatile() })
                    } else {
                        None
                    };
                    (lsr_val, byte)
                }
            };
            let _ = lsr;
            match rhr_byte {
                None => break,
                Some(c) => {
                    let c = if c == b'\r' { b'\n' } else { c };
                    buf.push_back(c);
                }
            }
        }
    }
}
