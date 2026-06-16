//! VirtIO network device driver.
//!
//! Discovers a VirtIO NIC in the MMIO range, initialises RX/TX virtqueues,
//! and exposes `send_frame` / `recv_frame` for the net service Cell.  Follows
//! the same MMIO probe pattern as `virtio_blk.rs`.

use crate::sync::Spinlock;
use crate::task::drivers::virtio_hal::VirtioHal;
use virtio_drivers::{
    device::net::VirtIONet,
    transport::mmio::{MmioTransport, VirtIOHeader},
    transport::{DeviceType, Transport},
};

/// RX/TX virtqueue size (frames). Power of two, ≤ the device's advertised
/// `max_queue_size` (QEMU reports 1024).
const NET_QUEUE_SIZE: usize = 16;
/// RX buffer length passed to `VirtIONet::new`. Must be ≥ virtio-drivers'
/// `MIN_BUFFER_LEN` (1526 = 1514 max Ethernet frame + 12-byte VirtioNetHdr);
/// a smaller value makes `receive_begin` reject every buffer with
/// `InvalidParam`. Round up to 2048 for headroom.
const RX_BUFFER_LEN: usize = 2048;

type SafeNet = VirtIONet<VirtioHal, MmioTransport, NET_QUEUE_SIZE>;
struct SafeVirtIONet(SafeNet);

// SAFETY: SafeVirtIONet is only ever accessed under the Spinlock which
// serialises access from a single-core kernel.
unsafe impl Send for SafeVirtIONet {}
unsafe impl Sync for SafeVirtIONet {}

static NET_DEVICE: Spinlock<Option<SafeVirtIONet>> = Spinlock::new(None);
static NET_IRQ:    Spinlock<u32>                   = Spinlock::new(0);

/// Probe all VirtIO MMIO slots for a NIC and initialise it.
///
/// Uses `virtio_slots()` for platform-correct slot enumeration (covers all
/// 32 AArch64 slots at 0x0a000000 and RISC-V DTB-confirmed slots).
/// Must be called once during kernel boot (`drivers::init()`).
pub fn init_driver() {
    use crate::task::drivers::virtio_common::virtio_slots;
    for slot in virtio_slots() {
        // SAFETY: slot.base is an identity-mapped VirtIO MMIO address.
        let header = unsafe {
            core::ptr::NonNull::new_unchecked(slot.base as *mut VirtIOHeader)
        };
        // SAFETY: MmioTransport::new validates magic/version before use.
        let Ok(transport) = (unsafe { MmioTransport::new(header) }) else { continue };
        if transport.device_type() != DeviceType::Network {
            // MmioTransport::drop() resets the device via set_status(0). Forget
            // the transport to avoid resetting a slot owned by another driver.
            core::mem::forget(transport);
            continue;
        }

        match VirtIONet::<VirtioHal, MmioTransport, NET_QUEUE_SIZE>::new(transport, RX_BUFFER_LEN) {
            Ok(net) => {
                *NET_DEVICE.lock() = Some(SafeVirtIONet(net));
                *NET_IRQ.lock() = slot.irq;
                log::info!("[virtio_net] NIC found at {:#x} irq={}", slot.base, slot.irq);
                return;
            }
            Err(e) => {
                log::error!("[virtio_net] VirtIONet::new failed at {:#x}: {:?}", slot.base, e);
            }
        }
    }
    log::warn!("[virtio_net] No VirtIO NIC found in MMIO range");
}

/// Transmit one Ethernet frame.
///
/// `frame` must include the Ethernet header; the VirtIO net header is
/// prepended internally by the driver.
///
/// # Errors
/// Returns `false` if the device is not initialised or the TX ring is full.
pub fn send_frame(frame: &[u8]) -> bool {
    let mut guard = NET_DEVICE.lock();
    if let Some(SafeVirtIONet(ref mut net)) = *guard {
        // Allocate a driver-managed TX buffer, fill it, hand it back for DMA.
        let mut tx_buf = net.new_tx_buffer(frame.len());
        tx_buf.packet_mut().copy_from_slice(frame);
        net.send(tx_buf).is_ok()
    } else {
        false
    }
}

/// Receive one Ethernet frame into `buf`.
///
/// Returns the number of bytes written to `buf`, or 0 if no frame is ready.
pub fn recv_frame(buf: &mut [u8]) -> usize {
    let mut guard = NET_DEVICE.lock();
    let Some(SafeVirtIONet(ref mut net)) = *guard else { return 0 };
    match net.receive() {
        Ok(rx_buf) => {
            let len = rx_buf.packet_len().min(buf.len());
            buf[..len].copy_from_slice(&rx_buf.packet()[..len]);
            if let Err(e) = net.recycle_rx_buffer(rx_buf) {
                log::warn!("[virtio_net] recycle_rx_buffer failed: {:?}", e);
            }
            len
        }
        Err(_) => 0,
    }
}

/// Handle the VirtIO net IRQ — acknowledges the interrupt and signals the waker.
///
/// The waker sets `NET_RX_PENDING` and pends SSIP so the timer sweep immediately
/// wakes any net cell parked in `WaitForEvent(NET_RX)` (Phase 04).
pub fn handle_irq() {
    let mut guard = NET_DEVICE.lock();
    if let Some(SafeVirtIONet(ref mut net)) = *guard {
        net.ack_interrupt();
    }
    drop(guard);
    crate::task::waker::signal_net_rx();
}

/// Return `true` if `irq` belongs to the VirtIO NIC and handle it.
///
/// Follows the same pattern as `virtio_input::ack_irq` — checks the stored
/// IRQ number internally so `vi_handle_virtio_irq` stays architecture-agnostic.
pub fn ack_irq(irq: u32) -> bool {
    let net_irq = *NET_IRQ.lock();
    if net_irq == 0 || net_irq != irq {
        return false;
    }
    handle_irq();
    true
}

/// Return the MAC address of the discovered NIC, or an all-zero address if
/// no device is present.
pub fn mac_address() -> [u8; 6] {
    let guard = NET_DEVICE.lock();
    if let Some(SafeVirtIONet(ref net)) = *guard {
        net.mac_address()
    } else {
        [0u8; 6]
    }
}
