use crate::sync::Spinlock;
use crate::task::drivers::virtio_hal::VirtioHal;
use virtio_drivers::device::blk::VirtIOBlk;
use virtio_drivers::transport::mmio::{MmioTransport, VirtIOHeader};
use virtio_drivers::transport::{DeviceType, Transport};

const VIRTIO0: usize = 0x10001000;
const VIRTIO_MMIO_INTERVAL: usize = 0x1000;
const VIRTIO_MAX_DEVICES: usize = 8;

pub struct SafeVirtIOBlk(VirtIOBlk<VirtioHal, MmioTransport>);
unsafe impl Send for SafeVirtIOBlk {}
unsafe impl Sync for SafeVirtIOBlk {}

pub static BLOCK_DEVICE: Spinlock<Option<SafeVirtIOBlk>> = Spinlock::new(None);
/// IRQ number assigned to the block device during probing (slot_index + 1 for QEMU VirtIO MMIO).
static BLOCK_DEVICE_IRQ: Spinlock<u32> = Spinlock::new(0);

pub fn init_driver() {
    log::debug!("VirtIO Block: probing MMIO slots...");

    for i in 0..VIRTIO_MAX_DEVICES {
        let addr = VIRTIO0 + i * VIRTIO_MMIO_INTERVAL;
        // SAFETY: addr is a QEMU virt MMIO slot that is identity-mapped by
        // init_kernel_paging; the pointer is valid for the VirtIOHeader layout.
        let header = unsafe { core::ptr::NonNull::new_unchecked(addr as *mut VirtIOHeader) };

        // SAFETY: header points to a valid VirtIO MMIO region (identity-mapped above).
        // MmioTransport::new reads the magic/version fields to validate the device;
        // an invalid slot returns Err without memory-safety issues.
        match unsafe { MmioTransport::new(header) } {
            Ok(transport) => {
                if transport.device_type() == DeviceType::Block {
                    match VirtIOBlk::<VirtioHal, MmioTransport>::new(transport) {
                        Ok(blk) => {
                            let mut locked_dev = BLOCK_DEVICE.lock();
                            *locked_dev = Some(SafeVirtIOBlk(blk));
                            // Record which IRQ this slot maps to (QEMU: slot i → IRQ i+1).
                            *BLOCK_DEVICE_IRQ.lock() = (i as u32) + 1;
                            log::info!("VirtIO Block: initialized at MMIO slot {}", i);
                            return; // Only support 1 block device for now
                        }
                        Err(e) => {
                            log::warn!("VirtIO Block: init error at slot {}: {:?}", i, e);
                        }
                    }
                }
            }
            Err(_) => {
                // Ignore invalid devices
            }
        }
    }
    log::debug!("VirtIO Block: no device found in MMIO range");
}

/// Called from the trap handler when any VirtIO MMIO IRQ fires (IRQs 1-8).
///
/// Dispatches to the matching device and calls `ack_interrupt()` to clear the
/// device's `InterruptStatus` register.  Without this call the device's IRQ line
/// stays asserted, the PLIC re-fires the interrupt immediately after `plic_complete`,
/// creating an interrupt storm that deadlocks all polling loops.
#[no_mangle]
pub extern "Rust" fn vi_handle_virtio_irq(irq: u32) {
    // --- Block device ---
    let block_irq = *BLOCK_DEVICE_IRQ.lock();
    if block_irq != 0 && block_irq == irq {
        let mut dev_lock = BLOCK_DEVICE.lock();
        if let Some(dev) = dev_lock.as_mut() {
            dev.0.ack_interrupt();
        }
        return;
    }

    // --- Input (keyboard) device ---
    // ack_irq clears InterruptStatus; without this an input IRQ becomes a storm.
    if crate::task::drivers::virtio_input::ack_irq(irq) {
        return;
    }

    // Unknown VirtIO slot — no device registered for this IRQ.
    // InterruptStatus is already cleared by plic_complete in the trap handler.
    log::warn!("[virtio] unhandled IRQ {} — no registered device for this slot", irq);
}

use api::block::ViBlockDevice;
use types::{ViError, ViResult};

#[allow(non_camel_case_types)]
pub struct viVirtIOBlk;

/// Warn if `read_blocks`/`write_blocks` takes longer than this many hardware
/// timer ticks without a response.  On QEMU virt the timer CSR runs at 10 MHz,
/// so 10 M ticks ≈ 1 second — a healthy read finishes in microseconds.
/// Triggered by an unattached disk image, unmapped MMIO, or virtqueue misconfiguration.
const POLL_WARN_TICKS: usize = 10_000_000;

impl ViBlockDevice for viVirtIOBlk {
    fn read_sector(&self, sector: u64, buf: &mut [u8]) -> ViResult<()> {
        let mut dev_lock = BLOCK_DEVICE.lock();
        let Some(dev) = dev_lock.as_mut() else {
            return Err(ViError::NotFound);
        };

        // read_blocks spins on the used ring (no IRQ sleep).  The BLOCK_DEVICE
        // Spinlock disables interrupts for the duration, but QEMU updates the
        // used ring via emulated DMA regardless of interrupt state, so polling
        // converges without needing an IRQ delivery.
        //
        // Defensive: log once if the spin-count suggests a hung device (e.g.,
        // disk image not attached to QEMU command line, or MMIO not mapped).
        // Use the hardware TIME CSR (monotonic, no software ticker needed).
        let t0 = riscv::register::time::read();
        let result = dev.0.read_blocks(sector as usize, buf);
        let elapsed = riscv::register::time::read().wrapping_sub(t0);
        if elapsed > POLL_WARN_TICKS {
            log::warn!(
                "[virtio-blk] read sector {} took {} ticks — possible hang (no disk image?)",
                sector, elapsed
            );
        }

        result.map_err(|e| {
            log::error!("[virtio-blk] read_blocks sector {}: {:?}", sector, e);
            ViError::NotFound
        })
    }

    fn write_sector(&self, sector: u64, buf: &[u8]) -> ViResult<()> {
        let mut dev_lock = BLOCK_DEVICE.lock();
        let Some(dev) = dev_lock.as_mut() else {
            return Err(ViError::NotFound);
        };

        let t0 = riscv::register::time::read();
        let result = dev.0.write_blocks(sector as usize, buf);
        let elapsed = riscv::register::time::read().wrapping_sub(t0);
        if elapsed > POLL_WARN_TICKS {
            log::warn!(
                "[virtio-blk] write sector {} took {} ticks — possible hang (no disk image?)",
                sector, elapsed
            );
        }

        result.map_err(|e| {
            log::error!("[virtio-blk] write_blocks sector {}: {:?}", sector, e);
            ViError::NotFound
        })
    }

    fn sector_count(&self) -> u64 {
        let mut dev_lock = BLOCK_DEVICE.lock();
        if let Some(dev) = dev_lock.as_mut() {
            dev.0.capacity()
        } else {
            0
        }
    }

    fn sector_size(&self) -> usize {
        512 // VirtIO standard usually
    }

    fn flush(&self) -> ViResult<()> {
        // VirtIO block has no explicit flush command in the spec; a write_blocks call
        // is already synchronous (waits for the device to complete the request).
        // Return NotFound if the device was never probed, so callers don't mistake
        // an absent device for successful durability.
        if BLOCK_DEVICE.lock().is_some() {
            Ok(())
        } else {
            Err(ViError::NotFound)
        }
    }
}
