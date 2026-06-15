//! virtio-net device model (DeviceID=1, virtio-mmio slot 2 → SPI 18).
//!
//! TX: guest writes TX avail ring → VMM reads chain → L2Send to Net Cell → kernel NIC TX.
//! RX: Net Cell L2Recv delivers frame → VMM fills RX avail descriptor → guest SPI 18.
//!
//! The 10-byte virtio_net_hdr is prepended on RX and stripped on TX.

extern crate alloc;
use alloc::vec;
use crate::virtio_mmio::{QueueCfg, VirtioDevice, VirtioMmio};
use crate::virtqueue::{process_notify, DescBuf};

/// MAC address presented to the guest virtio-net device.
pub const GUEST_MAC: [u8; 6] = [0x52, 0x54, 0x00, 0xAA, 0xBB, 0xCC];

/// GICv2 SPI for virtio-mmio slot 2 (intid 18, matches DTB and MMIO_HOLES pre-map).
const NET_SPI: u32 = 18;

/// virtio-net feature bit: device provides a MAC address in config space.
const VIRTIO_NET_F_MAC: u32 = 1 << 5;

pub struct NetDev {
    /// Net Cell TID for L2 frame IPC; 0 = net service unavailable.
    pub net_tid:       usize,
    rx_last_avail:     u16,
    rx_used_idx:       u16,
    tx_last_avail:     u16,
    tx_used_idx:       u16,
}

impl NetDev {
    pub fn new(net_tid: usize) -> Self {
        Self { net_tid, rx_last_avail: 0, rx_used_idx: 0, tx_last_avail: 0, tx_used_idx: 0 }
    }

    /// Inject one received Ethernet frame into the guest RX virtqueue.
    ///
    /// Prepends a 10-byte zero virtio_net_hdr, fills one avail descriptor chain,
    /// advances the used ring, and injects SPI 18 to notify the guest.
    /// Silently drops the frame if the avail ring is empty (no guest buffers ready).
    pub fn push_rx_frame(
        &mut self,
        frame: &[u8],
        vm_id: usize,
        vcpu_id: usize,
        net_vmio: &VirtioMmio,
    ) {
        let qcfg = net_vmio.queue_cfg(0); // RX queue (queue 0)
        if qcfg.num == 0 || !qcfg.ready { return; }

        // Read avail.idx to check if the guest has empty buffers ready.
        let mut b2 = [0u8; 2];
        if crate::vmm::read_guest_memory(vm_id, qcfg.avail_gpa + 2, &mut b2) != 2 { return; }
        let avail_idx = u16::from_le_bytes(b2);
        if self.rx_last_avail == avail_idx { return; } // No RX buffers available

        // Get the head descriptor index from avail.ring[last_avail_idx % q_size].
        let ring_off = 4 + (self.rx_last_avail as usize % qcfg.num as usize) * 2;
        if crate::vmm::read_guest_memory(vm_id, qcfg.avail_gpa + ring_off as u64, &mut b2) != 2 {
            return;
        }
        let head = u16::from_le_bytes(b2) as usize;
        self.rx_last_avail = self.rx_last_avail.wrapping_add(1);

        // Build payload: 10-byte zero virtio_net_hdr followed by the raw Ethernet frame.
        let mut payload = vec![0u8; 10 + frame.len()];
        payload[10..].copy_from_slice(frame);

        // Walk descriptor chain and write payload into writable (VRING_DESC_F_WRITE) buffers.
        let mut pos = 0usize;
        let mut cur = head;
        for _ in 0..64 {
            let mut raw = [0u8; 16]; // VirtqDesc: addr(8) + len(4) + flags(2) + next(2)
            let desc_gpa = qcfg.desc_gpa + (cur as u64) * 16;
            if crate::vmm::read_guest_memory(vm_id, desc_gpa, &mut raw) != 16 { break; }

            let addr  = u64::from_le_bytes([raw[0], raw[1], raw[2], raw[3],
                                            raw[4], raw[5], raw[6], raw[7]]);
            let len   = u32::from_le_bytes([raw[8],  raw[9],  raw[10], raw[11]]) as usize;
            let flags = u16::from_le_bytes([raw[12], raw[13]]);
            let next  = u16::from_le_bytes([raw[14], raw[15]]) as usize;

            if flags & 2 != 0 { // VRING_DESC_F_WRITE
                let n = payload.len().saturating_sub(pos).min(len);
                if n > 0 {
                    crate::vmm::write_guest_memory(vm_id, addr, &payload[pos..pos + n]);
                    pos += n;
                }
            }
            if flags & 1 == 0 { break; } // No VRING_DESC_F_NEXT
            cur = next;
        }

        // Write used ring entry {id: head, len: bytes_written} and advance used.idx.
        let written = pos as u32;
        let elem_off = 4 + (self.rx_used_idx as usize % qcfg.num as usize) * 8;
        let mut elem = [0u8; 8];
        elem[0..4].copy_from_slice(&(head as u32).to_le_bytes());
        elem[4..8].copy_from_slice(&written.to_le_bytes());
        crate::vmm::write_guest_memory(vm_id, qcfg.used_gpa + elem_off as u64, &elem);
        self.rx_used_idx = self.rx_used_idx.wrapping_add(1);
        crate::vmm::write_guest_memory(vm_id, qcfg.used_gpa + 2, &self.rx_used_idx.to_le_bytes());

        crate::vmm::inject_irq(vm_id, vcpu_id, NET_SPI);
    }
}

impl VirtioDevice for NetDev {
    fn device_id(&self) -> u32 { 1 } // DeviceID=1 = virtio-net

    fn device_features_lo(&self) -> u32 { VIRTIO_NET_F_MAC }

    /// Config space: MAC[0..5] at bytes 0-5, status at bytes 6-7.
    fn config_read(&self, offset: usize) -> u32 {
        match offset {
            0 => u32::from_le_bytes([GUEST_MAC[0], GUEST_MAC[1], GUEST_MAC[2], GUEST_MAC[3]]),
            4 => u32::from_le_bytes([GUEST_MAC[4], GUEST_MAC[5], 0, 0]),
            _ => 0,
        }
    }

    fn notify(&mut self, q: usize, qcfg: &QueueCfg, vm_id: usize, vcpu_id: usize) {
        match q {
            0 => {} // RX queue notify — guest added empty buffers; no action until frame arrives.
            1 => {
                // TX queue — drain guest TX descriptors and forward to the Net Cell.
                let net_tid = self.net_tid;
                process_notify(
                    vm_id, qcfg,
                    &mut self.tx_last_avail, &mut self.tx_used_idx,
                    |bufs| { handle_tx(bufs, vm_id, net_tid); 0 },
                );
                crate::vmm::inject_irq(vm_id, vcpu_id, NET_SPI);
            }
            _ => {}
        }
    }
}

/// Read all device-readable descriptor bytes, skip 10-byte virtio_net_hdr, send remainder.
fn handle_tx(bufs: &[DescBuf], vm_id: usize, net_tid: usize) {
    if net_tid == 0 { return; }
    let mut payload: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    for buf in bufs {
        if buf.writable { continue; } // Skip device-write buffers (not present in TX)
        let n = buf.len as usize;
        let mut tmp = vec![0u8; n];
        let got = crate::vmm::read_guest_memory(vm_id, buf.gpa, &mut tmp);
        if got == 0 || got == usize::MAX { return; }
        payload.extend_from_slice(&tmp[..got]);
    }
    // The first 10 bytes are the virtio_net_hdr; the rest is the raw Ethernet frame.
    if payload.len() <= 10 { return; }
    crate::net_backend::transmit(net_tid, &payload[10..]);
}
