//! virtio-console device model (DeviceID=3, virtio-mmio slot 0 → SPI 16).
//!
//! Queue 0 (rx): host → guest (stub — keyboard input is a future phase).
//! Queue 1 (tx): guest → host: drain TX buffers and print to ViCell serial.

extern crate alloc;
use alloc::vec;
use ostd::io::print;

use crate::virtio_mmio::{QueueCfg, VirtioDevice};
use crate::virtqueue::{process_notify, DescBuf};

/// SPI line for virtio-mmio slot 0.
const CONSOLE_SPI: u32 = 16;

pub struct Console {
    last_avail_rx: u16,
    last_avail_tx: u16,
    used_rx:       u16,
    used_tx:       u16,
}

impl Console {
    pub const fn new() -> Self {
        Self { last_avail_rx: 0, last_avail_tx: 0, used_rx: 0, used_tx: 0 }
    }
}

impl VirtioDevice for Console {
    fn device_id(&self) -> u32 { 3 }

    fn notify(&mut self, q: usize, qcfg: &QueueCfg, vm_id: usize, vcpu_id: usize) {
        match q {
            0 => {
                // RX queue (host→guest): stub.
                // A future phase will inject ViCell serial input as guest virtio-console input.
                let _ = (&mut self.last_avail_rx, &mut self.used_rx);
            }
            1 => {
                // TX queue (guest→host): read each readable descriptor and forward to serial.
                process_notify(
                    vm_id,
                    qcfg,
                    &mut self.last_avail_tx,
                    &mut self.used_tx,
                    |bufs: &[DescBuf]| {
                        let mut total = 0u32;
                        for buf in bufs {
                            if buf.writable { continue; } // TX bufs are driver-readable
                            let cap = buf.len.min(4096) as usize;
                            let mut data = vec![0u8; cap];
                            let n = crate::vmm::read_guest_memory(vm_id, buf.gpa, &mut data);
                            if n == 0 || n == usize::MAX { continue; }
                            if let Ok(s) = core::str::from_utf8(&data[..n]) {
                                print(s);
                            }
                            total += n as u32;
                        }
                        total
                    },
                );
                // Inject SPI so the guest interrupt handler runs and processes the used ring.
                crate::vmm::inject_irq(vm_id, vcpu_id, CONSOLE_SPI);
            }
            _ => {}
        }
    }
}
