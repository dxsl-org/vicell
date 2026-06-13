use crate::sync::Spinlock;
use crate::task::drivers::virtio_hal::VirtioHal;
use virtio_drivers::device::blk::VirtIOBlk;
use virtio_drivers::transport::mmio::{MmioTransport, VirtIOHeader};
use virtio_drivers::transport::{DeviceType, Transport};


pub struct SafeVirtIOBlk(VirtIOBlk<VirtioHal, MmioTransport>);
unsafe impl Send for SafeVirtIOBlk {}
unsafe impl Sync for SafeVirtIOBlk {}

pub static BLOCK_DEVICE: Spinlock<Option<SafeVirtIOBlk>> = Spinlock::new(None);
/// IRQ number assigned to the block device during probing (slot_index + 1 for QEMU VirtIO MMIO).
static BLOCK_DEVICE_IRQ: Spinlock<u32> = Spinlock::new(0);

/// Force-release this module's locks during fault teardown.
///
/// # Safety
/// Single-hart; called only from the fault/panic path with interrupts disabled.
pub unsafe fn force_unlock_locks() {
    BLOCK_DEVICE.force_unlock();
    BLOCK_DEVICE_IRQ.force_unlock();
}

pub fn init_driver() {
    use crate::task::drivers::virtio_common::virtio_slots;
    log::info!("VirtIO Block: probing MMIO slots...");

    for slot in virtio_slots() {
        // SAFETY: slot.base is an identity-mapped VirtIO MMIO address.
        let header = unsafe { core::ptr::NonNull::new_unchecked(slot.base as *mut VirtIOHeader) };
        match unsafe { MmioTransport::new(header) } {
            Ok(transport) => {
                if transport.device_type() == DeviceType::Block {
                    match VirtIOBlk::<VirtioHal, MmioTransport>::new(transport) {
                        Ok(blk) => {
                            *BLOCK_DEVICE.lock() = Some(SafeVirtIOBlk(blk));
                            *BLOCK_DEVICE_IRQ.lock() = slot.irq;
                            log::info!("VirtIO Block: initialized at {:#x} irq={}", slot.base, slot.irq);
                            return;
                        }
                        Err(e) => {
                            log::warn!("VirtIO Block: init error at {:#x}: {:?}", slot.base, e);
                        }
                    }
                } else {
                    // MmioTransport::drop() resets the device via set_status(0). Forget
                    // the transport to avoid resetting a slot owned by another driver.
                    core::mem::forget(transport);
                }
            }
            Err(_) => {}
        }
    }
    log::info!("VirtIO Block: no device found");
}

/// Returns `true` when a VirtIO block device was successfully probed.
pub fn is_present() -> bool {
    BLOCK_DEVICE.lock().is_some()
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
    // After ACKing, call poll_events() to drain pending events into event_queue so
    // they are ready when the shell next calls sys_read(0, ...).
    if crate::task::drivers::virtio_input::ack_irq(irq) {
        crate::task::drivers::virtio_input::poll_events();
        crate::task::drivers::virtio_input::dispatch_pending();
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
        #[cfg(target_arch = "riscv64")]
        let t0 = riscv::register::time::read();
        #[cfg(not(target_arch = "riscv64"))]
        let t0 = 0usize;
        let result = dev.0.read_blocks(sector as usize, buf);
        #[cfg(target_arch = "riscv64")]
        let elapsed = riscv::register::time::read().wrapping_sub(t0);
        #[cfg(not(target_arch = "riscv64"))]
        let elapsed = 0usize;
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

        #[cfg(target_arch = "riscv64")]
        let t0 = riscv::register::time::read();
        #[cfg(not(target_arch = "riscv64"))]
        let t0 = 0usize;
        let result = dev.0.write_blocks(sector as usize, buf);
        #[cfg(target_arch = "riscv64")]
        let elapsed = riscv::register::time::read().wrapping_sub(t0);
        #[cfg(not(target_arch = "riscv64"))]
        let elapsed = 0usize;
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
        let mut dev_lock = BLOCK_DEVICE.lock();
        let Some(dev) = dev_lock.as_mut() else {
            return Err(ViError::NotFound);
        };
        // Send a VirtIO FLUSH request (VIRTIO_BLK_T_FLUSH) so the device
        // commits all pending writes to the backing storage. Required for
        // reboot persistence: without a flush, QEMU may still hold dirty
        // data in its write-back buffer when the guest powers off.
        dev.0.flush().map_err(|e| {
            log::error!("[virtio-blk] flush: {:?}", e);
            ViError::NotFound
        })
    }
}
