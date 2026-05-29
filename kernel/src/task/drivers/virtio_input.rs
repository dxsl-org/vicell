use crate::sync::Spinlock;
use crate::task::drivers::virtio_hal::VirtioHal;
use alloc::collections::VecDeque;
use core::ptr::NonNull;
use virtio_drivers::device::input::VirtIOInput;
use virtio_drivers::transport::mmio::{MmioTransport, VirtIOHeader};
use virtio_drivers::transport::{DeviceType, Transport};

pub struct KeyboardEvent {
    pub event_type: u16,
    pub code: u16,
    pub value: u32,
}

pub struct VirtIOInputDriver {
    pub input: VirtIOInput<VirtioHal, MmioTransport>,
    pub event_queue: VecDeque<KeyboardEvent>,
}

pub static KEYBOARD_DRIVER: Spinlock<Option<VirtIOInputDriver>> = Spinlock::new(None);

/// IRQ number of the probed input device (0 = not found).
/// Set during init so the interrupt handler can identify this device's IRQ.
pub static INPUT_DEVICE_IRQ: Spinlock<u32> = Spinlock::new(0);

pub fn init_driver() {
    for i in 0..8 {
        let addr = 0x1000_1000 + i * 0x1000;
        let header = unsafe { NonNull::new_unchecked(addr as *mut VirtIOHeader) };
        match unsafe { MmioTransport::new(header) } {
            Ok(transport) => {
                if transport.device_type() == DeviceType::Input {
                    match VirtIOInput::<VirtioHal, MmioTransport>::new(transport) {
                        Ok(input) => {
                            log::info!("VirtIO Input: initialized at MMIO slot {}", i);
                            // Record the IRQ number (QEMU VirtIO MMIO: slot i → IRQ i+1).
                            *INPUT_DEVICE_IRQ.lock() = (i as u32) + 1;
                            *KEYBOARD_DRIVER.lock() = Some(VirtIOInputDriver {
                                input,
                                event_queue: VecDeque::new(),
                            });
                            return;
                        }
                        Err(e) => log::warn!("VirtIO Input init failed: {:?}", e),
                    }
                }
            }
            Err(_) => {}
        }
    }
}

/// Called from the trap handler when a VirtIO IRQ fires.
///
/// Returns `true` if the IRQ belonged to the input device and was acknowledged.
/// Failing to call this causes `InterruptStatus` to stay set, which makes the
/// PLIC re-fire the same IRQ immediately after `plic_complete` — an interrupt storm.
pub fn ack_irq(irq: u32) -> bool {
    let device_irq = *INPUT_DEVICE_IRQ.lock();
    if device_irq == 0 || device_irq != irq {
        return false;
    }
    if let Some(drv) = KEYBOARD_DRIVER.lock().as_mut() {
        drv.input.ack_interrupt();
    }
    true
}

pub fn poll_events() {
    if let Some(driver) = KEYBOARD_DRIVER.lock().as_mut() {
        while let Some(event) = driver.input.pop_pending_event() {
            log::debug!(
                "VirtIO Input Event: type={}, code={}, value={}",
                event.event_type,
                event.code,
                event.value
            );
            driver.event_queue.push_back(KeyboardEvent {
                event_type: event.event_type,
                code: event.code,
                value: event.value,
            });
        }
    }
}
