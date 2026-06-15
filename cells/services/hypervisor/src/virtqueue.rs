//! Split-virtqueue device-side ring processor.
//!
//! The VMM is on the device side: it reads the avail ring, walks desc chains,
//! hands buffers to the device model, then writes the used ring.
//! All guest memory accesses go via `crate::vmm` syscall wrappers — never raw
//! pointer dereference from the cell (Law 4).

extern crate alloc;
use alloc::vec::Vec;

use crate::virtio_mmio::QueueCfg;

/// One segment from a virtqueue descriptor chain.
pub struct DescBuf {
    pub gpa:      u64,
    pub len:      u32,
    /// true = device writes into this buffer (VRING_DESC_F_WRITE); false = device reads.
    pub writable: bool,
}

const FLAGS_NEXT:  u16 = 1;
const FLAGS_WRITE: u16 = 2;
const MAX_CHAIN:   usize = 64; // guard against infinite chains

/// Process one QueueNotify: drain avail ring → walk desc chains → call `handle` → update used ring.
///
/// `last_avail_idx` and `used_idx` are per-queue device-side counters (NOT in guest memory);
/// the caller (device model) owns them and passes them mutably across calls.
pub fn process_notify<F>(
    vm_id: usize,
    qcfg: &QueueCfg,
    last_avail_idx: &mut u16,
    used_idx: &mut u16,
    mut handle: F,
) where
    F: FnMut(&[DescBuf]) -> u32,
{
    let q_size = qcfg.num as usize;
    if q_size == 0 { return; }

    // Read avail.idx (u16 at avail_ring + 2).
    let mut b2 = [0u8; 2];
    if crate::vmm::read_guest_memory(vm_id, qcfg.avail_gpa + 2, &mut b2) != 2 { return; }
    let avail_idx = u16::from_le_bytes(b2);

    while *last_avail_idx != avail_idx {
        // Read avail.ring[last_avail_idx % q_size] — the desc head index.
        let ring_off = 4 + (*last_avail_idx as usize % q_size) * 2;
        if crate::vmm::read_guest_memory(vm_id, qcfg.avail_gpa + ring_off as u64, &mut b2) != 2 {
            break;
        }
        let head = u16::from_le_bytes(b2) as usize;
        *last_avail_idx = last_avail_idx.wrapping_add(1);

        // Walk descriptor chain starting at `head`.
        let mut bufs: Vec<DescBuf> = Vec::new();
        let mut cur = head;
        for _ in 0..MAX_CHAIN {
            let mut raw = [0u8; 16]; // VirtqDesc = 16 bytes
            let desc_gpa = qcfg.desc_gpa + (cur as u64) * 16;
            if crate::vmm::read_guest_memory(vm_id, desc_gpa, &mut raw) != 16 { break; }

            let addr  = u64::from_le_bytes([raw[0], raw[1], raw[2], raw[3],
                                            raw[4], raw[5], raw[6], raw[7]]);
            let len   = u32::from_le_bytes([raw[8],  raw[9],  raw[10], raw[11]]);
            let flags = u16::from_le_bytes([raw[12], raw[13]]);
            let next  = u16::from_le_bytes([raw[14], raw[15]]) as usize;

            bufs.push(DescBuf { gpa: addr, len, writable: flags & FLAGS_WRITE != 0 });
            if flags & FLAGS_NEXT == 0 { break; }
            cur = next;
        }

        // Deliver buffers to device model; it returns how many bytes were produced.
        let written = handle(&bufs);

        // Write used ring entry { id: u32, len: u32 } at used.ring[used_idx % q_size].
        let elem_off = 4 + (*used_idx as usize % q_size) * 8;
        let mut elem = [0u8; 8];
        elem[0..4].copy_from_slice(&(head as u32).to_le_bytes());
        elem[4..8].copy_from_slice(&written.to_le_bytes());
        crate::vmm::write_guest_memory(vm_id, qcfg.used_gpa + elem_off as u64, &elem);

        // Advance used.idx with a store that the guest will see (TCG is SC).
        *used_idx = used_idx.wrapping_add(1);
        crate::vmm::write_guest_memory(vm_id, qcfg.used_gpa + 2, &used_idx.to_le_bytes());
    }
}
