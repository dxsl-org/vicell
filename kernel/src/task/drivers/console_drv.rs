use crate::sync::Spinlock;
use alloc::collections::VecDeque;

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

    /// Polls SBI for a character and pushes it to buffer if available.
    /// Returns true if a character was received.
    pub fn poll(&mut self) -> bool {
        let mut received = false;

        // 1. Poll SBI/UART (Physical Serial) — primary input for -nographic QEMU.
        let c = crate::hal::sbi::console_getchar();
        if c > 0 {
            log::trace!("Console: UART byte {}", c); // trace-level: don't flood on every keystroke
            self.buffer.push_back(c as u8);
            received = true;
        }

        // 2. Poll VirtIO Keyboard — used when a graphical display is attached.
        crate::task::drivers::virtio_input::poll_events();
        if let Some(drv) = crate::task::drivers::virtio_input::KEYBOARD_DRIVER
            .lock()
            .as_mut()
        {
            while let Some(event) = drv.event_queue.pop_front() {
                if event.event_type == crate::task::drivers::input_map::EV_KEY {
                    if let Some(c) =
                        crate::task::drivers::input_map::scancode_to_ascii(event.code, event.value)
                    {
                        if c as u8 > 0 {
                            log::trace!("Console: VirtIO key {}", c);
                            self.buffer.push_back(c as u8);
                            received = true;
                        }
                    }
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
