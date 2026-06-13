use crate::sync::Spinlock;
use alloc::collections::VecDeque;
use core::sync::atomic::Ordering;

#[allow(non_camel_case_types)]
pub struct viConsole {
    pub buffer: VecDeque<u8>,
}

impl viConsole {
    pub fn new() -> Self {
        Self {
            buffer: VecDeque::new(),
        }
    }

    /// Hard cap on buffered input bytes. A line-oriented console never needs
    /// more than this; the cap prevents a misbehaving input source (e.g. an
    /// SBI/IRQ path that returns phantom bytes every poll) from growing the
    /// VecDeque unboundedly and exhausting the kernel heap while a reader spins.
    const MAX_BUFFERED: usize = 4096;

    /// Polls input sources and pushes available characters to the buffer.
    /// Returns true if a character was received.
    pub fn poll(&mut self) -> bool {
        // Already have plenty buffered — don't poll/push more until it drains.
        if self.buffer.len() >= Self::MAX_BUFFERED {
            return false;
        }
        let mut received = false;

        // 1a. Directly poll the 16550 RHR — RISC-V QEMU virt only.
        // The 16550 lives at 0x10_000_000 on RISC-V; that address is not a
        // 16550 on AArch64 (UART is PL011 at 0x0900_0000). Reading it there
        // returns garbage (0xFF), causing continuous `?` spam.
        #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
        while self.buffer.len() < Self::MAX_BUFFERED {
            let Some(c) = crate::task::drivers::uart::poll_rhr() else { break };
            self.buffer.push_back(c);
            received = true;
        }

        // 1b. Drain any chars the UART IRQ handler buffered (when IRQs reach S-mode).
        // This path is also only relevant for RISC-V; on AArch64 IRQ-buffered
        // chars come through the PL011 path below.
        #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
        while self.buffer.len() < Self::MAX_BUFFERED {
            let Some(c) = crate::task::drivers::uart::getchar() else { break };
            self.buffer.push_back(c);
            received = true;
        }

        // 1c. Poll PL011 UART RX on AArch64.
        // QEMU virt maps PL011 at 0x0900_0000; `-serial tcp:...` connects its
        // TX/RX to the TCP socket used by the integration-test harness.
        #[cfg(target_arch = "aarch64")]
        while self.buffer.len() < Self::MAX_BUFFERED {
            let Some(c) = crate::hal::uart_pl011::poll_rx() else { break };
            self.buffer.push_back(c);
            received = true;
        }

        // 1d. Drain IRQ-filled RX buffer on x86_64.
        // vi_handle_uart_irq() (fired by IOAPIC IRQ 4 / IDT vector 0x24) pushes
        // COM1 bytes into uart::RX_BUFFER; we drain it here on every poll call
        // so the blocking file_read(fd=0) loop eventually finds a byte.
        #[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
        while self.buffer.len() < Self::MAX_BUFFERED {
            let Some(c) = crate::task::drivers::uart::getchar() else { break };
            self.buffer.push_back(c);
            received = true;
        }

        // NOTE: the SBI DBCN console-read fallback was removed — on this QEMU /
        // OpenSBI build it returns phantom bytes on every call, which (combined
        // with a spinning reader) grew the buffer without bound. The direct RHR
        // poll (1a) is the reliable UART input path.

        // 2. Poll VirtIO Keyboard — used when a graphical display is attached.
        crate::task::drivers::virtio_input::poll_events();
        let input_tid = crate::task::drivers::virtio_input::INPUT_CELL_ID
            .load(Ordering::Relaxed);
        if let Some(drv) = crate::task::drivers::virtio_input::KEYBOARD_DRIVER
            .lock()
            .as_mut()
        {
            while let Some(event) = drv.event_queue.pop_front() {
                use crate::task::drivers::input_map::{EV_KEY, EV_REL, EV_ABS};
                if event.event_type == EV_KEY {
                    // Forward raw event to input service.
                    // Wire format: [opcode:1=0x00][code:4 LE][value:4 LE]
                    if input_tid != 0 {
                        let mut msg = [0u8; 9]; // msg[0]=0 = EV_KEY opcode
                        msg[1..5].copy_from_slice(&(event.code as u32).to_le_bytes());
                        msg[5..9].copy_from_slice(&event.value.to_le_bytes());
                        let _ = crate::task::ipc_send(0, input_tid, msg.as_ptr() as usize, 9);
                    }
                    // UART ASCII fallback — keeps shell input working regardless of input service state.
                    if let Some(c) =
                        crate::task::drivers::input_map::scancode_to_ascii(event.code, event.value)
                    {
                        if c as u8 > 0 {
                            log::trace!("Console: VirtIO key {}", c);
                            self.buffer.push_back(c as u8);
                            received = true;
                        }
                    }
                } else if input_tid != 0 {
                    // EV_REL → opcode 1, EV_ABS → opcode 2; no UART fallback for mouse.
                    let opcode = if event.event_type == EV_REL { 1u8 }
                        else if event.event_type == EV_ABS { 2u8 }
                        else { continue };
                    let mut msg = [0u8; 9];
                    msg[0] = opcode;
                    msg[1..5].copy_from_slice(&(event.code as u32).to_le_bytes());
                    msg[5..9].copy_from_slice(&event.value.to_le_bytes());
                    let _ = crate::task::ipc_send(0, input_tid, msg.as_ptr() as usize, 9);
                }
            }
        }

        received
    }

    /// Read a byte from buffer (Non-blocking)
    pub fn read_byte(&mut self) -> Option<u8> {
        self.buffer.pop_front()
    }
}

pub static CONSOLE: Spinlock<viConsole> = Spinlock::new(viConsole {
    buffer: VecDeque::new(),
});

pub fn init() {
    // Nothing special to init for SBI Console so far
    // But we might want to clear buffer
    let mut cons = CONSOLE.lock();
    cons.buffer.clear();
    log::info!("Console: Input Driver Initialized.");
}
