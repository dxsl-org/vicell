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

/// Same MMIO base and stride as VirtIO block (QEMU virt machine).
const VIRTIO0: usize = 0x10001000;
const VIRTIO_MMIO_INTERVAL: usize = 0x1000;
const VIRTIO_MAX_DEVICES: usize = 8;

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

/// Probe the MMIO range for a VirtIO NIC and initialise it.
///
/// Should be called once during kernel boot (`drivers::init()`).
pub fn init_driver() {
    for i in 0..VIRTIO_MAX_DEVICES {
        let addr = VIRTIO0 + i * VIRTIO_MMIO_INTERVAL;
        // SAFETY: addr is a valid QEMU virt MMIO slot (identity-mapped by init_kernel_paging).
        let header = unsafe {
            core::ptr::NonNull::new_unchecked(addr as *mut VirtIOHeader)
        };
        // SAFETY: same invariant as header; MmioTransport::new validates magic/version.
        let Ok(transport) = (unsafe { MmioTransport::new(header) }) else { continue };
        if transport.device_type() != DeviceType::Network {
            // Dropping MmioTransport resets the device via set_status(0). This
            // probe runs after the block driver is initialised, so resetting a
            // foreign slot (e.g. the block device) would corrupt it. Forget the
            // transport to skip the Drop, matching the GPU probe's approach.
            core::mem::forget(transport);
            continue;
        }

        match VirtIONet::<VirtioHal, MmioTransport, NET_QUEUE_SIZE>::new(transport, RX_BUFFER_LEN) {
            Ok(net) => {
                *NET_DEVICE.lock() = Some(SafeVirtIONet(net));
                *NET_IRQ.lock() = (i as u32) + 1;
                log::info!("[virtio_net] NIC found at MMIO slot {}, IRQ {}", i, i + 1);
                return;
            }
            Err(e) => {
                log::error!("[virtio_net] VirtIONet::new failed: {:?}", e);
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
