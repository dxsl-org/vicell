//! Intel e1000 (82540EM) PCIe NIC kernel driver — polled TX/RX.
//!
//! Discovers the e1000 via PCIe ECAM class 0x02/0x00, verifies vendor/device
//! IDs, maps BAR0 MMIO, sets up 16-entry TX and RX descriptor rings, reads the
//! MAC from the EEPROM, and enables the controller.
//!
//! TX is polled (wait for DD bit). RX is polled (check DD on next expected slot).
//! This is sufficient for the single-NIC, single-cell net service architecture.

use alloc::alloc::{alloc_zeroed, dealloc, Layout};
use core::sync::atomic::{compiler_fence, Ordering};

use crate::task::drivers::pcie_ecam;

// ── PCI identification ────────────────────────────────────────────────────────

const ETH_CLASS:  u8  = 0x02;
const ETH_SUB:    u8  = 0x00;
const ETH_PROGIF: u8  = 0x00;

const INTEL_VEN:   u16 = 0x8086;
const E1000_DEV:   u16 = 0x100E; // 82540EM (QEMU `-device e1000`)

// Additional NICs recognised (ID table only — same init code path)
const RTL8125_VEN: u16 = 0x10EC;
const RTL8125_DEV: u16 = 0x8125;
const I225_DEV:    u16 = 0x15F3;

// ── e1000 register offsets (in BAR0 MMIO, byte offsets) ──────────────────────

const CTRL:  usize = 0x0000;
const ICR:   usize = 0x00C0;
const IMC:   usize = 0x00D8;
const RCTL:  usize = 0x0100;
const TCTL:  usize = 0x0400;
const TIPG:  usize = 0x0410;
const RDBAL: usize = 0x2800;
const RDBAH: usize = 0x2804;
const RDLEN: usize = 0x2808;
const RDH:   usize = 0x2810;
const RDT:   usize = 0x2818;
const TDBAL: usize = 0x3800;
const TDBAH: usize = 0x3804;
const TDLEN: usize = 0x3808;
const TDH:   usize = 0x3810;
const TDT:   usize = 0x3818;
const RAL0:  usize = 0x5400;
const RAH0:  usize = 0x5404;
const MTA:   usize = 0x5200; // 128 × u32 = 512 B
const EERD:  usize = 0x0014;

// ── Register field constants ──────────────────────────────────────────────────

// CTRL
const CTRL_RST:  u32 = 1 << 26;
const CTRL_SLU:  u32 = 1 << 6;
const CTRL_ASDE: u32 = 1 << 5;

// RCTL
const RCTL_EN:   u32 = 1 << 1;
const RCTL_UPE:  u32 = 1 << 3;  // unicast promiscuous
const RCTL_MPE:  u32 = 1 << 4;  // multicast promiscuous
const RCTL_BAM:  u32 = 1 << 15; // broadcast accept
const RCTL_SECRC:u32 = 1 << 26; // strip CRC

// TCTL
const TCTL_EN:   u32 = 1 << 1;
const TCTL_PSP:  u32 = 1 << 3;
const TCTL_CT:   u32 = 0x0F << 4;  // collision threshold
const TCTL_COLD: u32 = 0x40 << 12; // collision distance

// TX descriptor cmd field
const CMD_EOP:  u8 = 0x01; // end of packet
const CMD_IFCS: u8 = 0x02; // insert FCS
const CMD_RS:   u8 = 0x08; // report status

// TX/RX descriptor status
const STATUS_DD: u8 = 0x01; // descriptor done

// EERD (EEPROM read)
const EERD_START: u32 = 1;
const EERD_DONE:  u32 = 1 << 4;

// RAH0 address-valid bit
const RAH_AV: u32 = 1 << 31;

// ── Descriptor types ──────────────────────────────────────────────────────────

const N_TX: usize = 16;
const N_RX: usize = 16;
const BUF_SIZE: usize = 2048;

// Fields are naturally aligned (u64@0, u16@8, u8@10..13, u16@14) → no packing needed.
#[repr(C)]
struct TxDesc {
    buf_addr:  u64,
    length:    u16,
    cso:       u8,
    cmd:       u8,
    status:    u8,
    css:       u8,
    special:   u16,
}

#[repr(C)]
struct RxDesc {
    buf_addr:  u64,
    length:    u16,
    checksum:  u16,
    status:    u8,
    errors:    u8,
    special:   u16,
}

const _: () = assert!(core::mem::size_of::<TxDesc>() == 16);
const _: () = assert!(core::mem::size_of::<RxDesc>() == 16);

// ── Driver state ──────────────────────────────────────────────────────────────

struct E1000 {
    bar0:     usize,
    tx_ring:  *mut TxDesc,
    rx_ring:  *mut RxDesc,
    tx_bufs:  [*mut u8; N_TX],
    rx_bufs:  [*mut u8; N_RX],
    tx_next:  usize, // next TX slot to use
    rx_head:  usize, // next RX slot to drain
}

// SAFETY: E1000 is only ever accessed behind the DEVICE Spinlock.
unsafe impl Send for E1000 {}

static DEVICE: crate::sync::Spinlock<Option<E1000>> = crate::sync::Spinlock::new(None);

// ── MMIO helpers ──────────────────────────────────────────────────────────────

#[inline]
unsafe fn rd32(base: usize, off: usize) -> u32 {
    // SAFETY: caller guarantees base+off is valid identity-mapped e1000 MMIO.
    unsafe { core::ptr::read_volatile((base + off) as *const u32) }
}

#[inline]
unsafe fn wr32(base: usize, off: usize, val: u32) {
    // SAFETY: caller guarantees base+off is valid identity-mapped e1000 MMIO.
    unsafe { core::ptr::write_volatile((base + off) as *mut u32, val) }
}

// ── DMA helpers ───────────────────────────────────────────────────────────────

#[inline]
fn dma_phys(virt: usize) -> u64 {
    #[cfg(target_arch = "x86_64")]
    { (virt - crate::memory::frame::phys_to_virt(0)) as u64 }
    #[cfg(not(target_arch = "x86_64"))]
    { virt as u64 }
}

fn dma_alloc(size: usize) -> *mut u8 {
    let layout = Layout::from_size_align(size, 4096)
        .expect("e1000 DMA layout");
    // SAFETY: layout is valid and non-zero.
    let ptr = unsafe { alloc_zeroed(layout) };
    assert!(!ptr.is_null(), "[e1000] OOM: failed to allocate DMA buffer");
    super::iommu::map_dma(dma_phys(ptr as usize), size);
    ptr
}

// ── EEPROM read ───────────────────────────────────────────────────────────────

unsafe fn eeprom_read(bar0: usize, addr: u8) -> u16 {
    // SAFETY: bar0 is identity-mapped e1000 MMIO.
    unsafe { wr32(bar0, EERD, ((addr as u32) << 8) | EERD_START); }
    let mut n = 0u32;
    loop {
        // SAFETY: bar0 is identity-mapped e1000 MMIO.
        let v = unsafe { rd32(bar0, EERD) };
        if v & EERD_DONE != 0 { return (v >> 16) as u16; }
        n += 1;
        if n > 1_000_000 { break; }
        core::hint::spin_loop();
    }
    log::warn!("[e1000] EEPROM read timeout (addr={:#x})", addr);
    0
}

// ── Initialisation ────────────────────────────────────────────────────────────

pub fn init_driver() {
    // 1. Discover e1000 by Ethernet class; verify vendor/device.
    let dev = match pcie_ecam::find_class(ETH_CLASS, ETH_SUB, ETH_PROGIF) {
        Some(d)
            if (d.vendor_id == INTEL_VEN && d.device_id == E1000_DEV)
            || (d.vendor_id == INTEL_VEN && d.device_id == I225_DEV)
            || (d.vendor_id == RTL8125_VEN && d.device_id == RTL8125_DEV) => d,
        Some(d) => {
            log::info!(
                "[e1000] Ethernet device {:04x}:{:04x} not in e1000 ID table — skipping",
                d.vendor_id, d.device_id
            );
            return;
        }
        None => {
            log::info!("[e1000] no Ethernet device found on PCIe bus");
            return;
        }
    };

    let bar0_phys = dev.bars[0].base_addr() as usize;
    if bar0_phys == 0 {
        log::warn!("[e1000] BAR0 == 0 (firmware did not configure MMIO)");
        return;
    }

    log::info!(
        "[e1000] found {:04x}:{:04x} bar0={:#x}",
        dev.vendor_id, dev.device_id, bar0_phys
    );

    // 2. On x86_64, identity-map the BAR0 MMIO window (128 KiB).
    #[cfg(target_arch = "x86_64")]
    crate::memory::paging::map_mmio_x86(bar0_phys, 128 * 1024);

    // On RISC-V/AArch64, MMIO is identity-mapped by init_kernel_paging.
    let bar0 = bar0_phys; // VA == PA after identity-map

    // 3. Soft-reset the controller.
    // SAFETY: bar0 is identity-mapped e1000 MMIO (just mapped above on x86_64).
    unsafe { wr32(bar0, CTRL, CTRL_RST); }
    for _ in 0..10_000 { core::hint::spin_loop(); }
    // Wait for RST to self-clear.
    let mut n = 0u32;
    loop {
        // SAFETY: bar0 is identity-mapped e1000 MMIO.
        if unsafe { rd32(bar0, CTRL) } & CTRL_RST == 0 { break; }
        n += 1;
        if n > 1_000_000 {
            log::warn!("[e1000] CTRL.RST stuck high after reset");
            return;
        }
        core::hint::spin_loop();
    }

    // 4. Set link-up bits.
    // SAFETY: bar0 is identity-mapped e1000 MMIO.
    unsafe { wr32(bar0, CTRL, CTRL_SLU | CTRL_ASDE); }

    // 5. Disable all interrupts.
    // SAFETY: bar0 is identity-mapped e1000 MMIO.
    unsafe {
        wr32(bar0, IMC, 0xFFFF_FFFF);
        let _ = rd32(bar0, ICR); // clear pending
    }

    // 6. Read MAC from EEPROM.
    // SAFETY: bar0 is identity-mapped e1000 MMIO.
    let mac_lo = unsafe { eeprom_read(bar0, 0) };
    let mac_hi = unsafe { eeprom_read(bar0, 1) };
    let mac_ex = unsafe { eeprom_read(bar0, 2) };
    let ral = (mac_hi as u32) << 16 | mac_lo as u32;
    let rah = RAH_AV | mac_ex as u32;
    // SAFETY: bar0 is identity-mapped e1000 MMIO.
    unsafe {
        wr32(bar0, RAL0, ral);
        wr32(bar0, RAH0, rah);
    }
    log::info!(
        "[e1000] MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac_lo as u8, (mac_lo >> 8) as u8,
        mac_hi as u8, (mac_hi >> 8) as u8,
        mac_ex as u8, (mac_ex >> 8) as u8,
    );

    // 7. Zero the multicast table (128 × u32).
    // SAFETY: bar0 is identity-mapped e1000 MMIO.
    for i in 0usize..128 {
        unsafe { wr32(bar0, MTA + i * 4, 0); }
    }

    // 8. Allocate TX ring + buffers.
    let tx_ring_size = N_TX * core::mem::size_of::<TxDesc>();
    let tx_ring = dma_alloc(tx_ring_size) as *mut TxDesc;
    let mut tx_bufs = [core::ptr::null_mut::<u8>(); N_TX];
    for slot in &mut tx_bufs {
        *slot = dma_alloc(BUF_SIZE);
    }

    // 9. Allocate RX ring + buffers.
    let rx_ring_size = N_RX * core::mem::size_of::<RxDesc>();
    let rx_ring = dma_alloc(rx_ring_size) as *mut RxDesc;
    let mut rx_bufs = [core::ptr::null_mut::<u8>(); N_RX];
    for slot in &mut rx_bufs {
        *slot = dma_alloc(BUF_SIZE);
    }

    // 10. Program TX descriptor ring.
    let tx_phys = dma_phys(tx_ring as usize);
    // SAFETY: bar0 is identity-mapped; tx_phys is 4096-aligned.
    unsafe {
        wr32(bar0, TDBAL, tx_phys as u32);
        wr32(bar0, TDBAH, (tx_phys >> 32) as u32);
        wr32(bar0, TDLEN, (N_TX * 16) as u32);
        wr32(bar0, TDH, 0);
        wr32(bar0, TDT, 0);
    }

    // 11. Fill RX descriptors with buffer addresses, program RX ring.
    for i in 0..N_RX {
        let phys = dma_phys(rx_bufs[i] as usize);
        // SAFETY: rx_ring[i] is a valid, zeroed TxDesc-sized slot.
        unsafe {
            let desc = &mut *rx_ring.add(i);
            core::ptr::write_volatile(&mut desc.buf_addr, phys);
            core::ptr::write_volatile(&mut desc.status, 0);
        }
    }
    let rx_phys = dma_phys(rx_ring as usize);
    // SAFETY: bar0 is identity-mapped; rx_phys is 4096-aligned.
    unsafe {
        wr32(bar0, RDBAL, rx_phys as u32);
        wr32(bar0, RDBAH, (rx_phys >> 32) as u32);
        wr32(bar0, RDLEN, (N_RX * 16) as u32);
        wr32(bar0, RDH, 0);
        wr32(bar0, RDT, (N_RX - 1) as u32); // give hardware N_RX-1 buffers
    }

    // 12. Configure TX: enable, pad short packets, set collision fields.
    // SAFETY: bar0 is identity-mapped e1000 MMIO.
    unsafe {
        wr32(bar0, TIPG, 0x0060_200A); // recommended TIPG for 802.3 (10 Mbit)
        wr32(bar0, TCTL, TCTL_EN | TCTL_PSP | TCTL_CT | TCTL_COLD);
    }

    // 13. Configure RX: enable, promiscuous + broadcast, strip CRC.
    // SAFETY: bar0 is identity-mapped e1000 MMIO.
    unsafe {
        wr32(bar0, RCTL, RCTL_EN | RCTL_UPE | RCTL_MPE | RCTL_BAM | RCTL_SECRC);
    }

    *DEVICE.lock() = Some(E1000 {
        bar0,
        tx_ring,
        rx_ring,
        tx_bufs,
        rx_bufs,
        tx_next: 0,
        rx_head: 0,
    });

    log::info!("[e1000] NIC initialized");
}

// ── Public API ────────────────────────────────────────────────────────────────

pub fn is_present() -> bool {
    DEVICE.lock().is_some()
}

/// Transmit `frame` (Ethernet frame including MAC header).
///
/// Blocks until the descriptor's DD bit is set (polled completion).
/// Returns `true` on success, `false` if no NIC is present or frame is too long.
pub fn send_frame(frame: &[u8]) -> bool {
    if frame.len() > BUF_SIZE { return false; }

    let mut guard = DEVICE.lock();
    let Some(nic) = guard.as_mut() else { return false; };

    let slot = nic.tx_next;
    nic.tx_next = (slot + 1) % N_TX;

    // Copy frame into the DMA buffer for this slot.
    // SAFETY: tx_bufs[slot] is a valid, DMA-allocated BUF_SIZE buffer.
    unsafe {
        core::ptr::copy_nonoverlapping(frame.as_ptr(), nic.tx_bufs[slot], frame.len());
    }

    let phys = dma_phys(nic.tx_bufs[slot] as usize);

    // SAFETY: tx_ring[slot] is a valid, DMA-allocated TxDesc slot.
    unsafe {
        let desc = &mut *nic.tx_ring.add(slot);
        core::ptr::write_volatile(&mut desc.buf_addr, phys);
        core::ptr::write_volatile(&mut desc.length, frame.len() as u16);
        core::ptr::write_volatile(&mut desc.cso, 0);
        core::ptr::write_volatile(&mut desc.cmd, CMD_EOP | CMD_IFCS | CMD_RS);
        core::ptr::write_volatile(&mut desc.status, 0); // clear DD before submit
        core::ptr::write_volatile(&mut desc.css, 0);
        core::ptr::write_volatile(&mut desc.special, 0);
    }

    compiler_fence(Ordering::Release);

    // Ring the TX doorbell.
    // SAFETY: bar0 is identity-mapped e1000 MMIO.
    unsafe { wr32(nic.bar0, TDT, nic.tx_next as u32); }

    // Poll DD (descriptor done) on the slot we just submitted.
    let mut n = 0u32;
    loop {
        compiler_fence(Ordering::Acquire);
        // SAFETY: tx_ring[slot] is a valid DMA-allocated TxDesc.
        let status = unsafe { core::ptr::read_volatile(&(*nic.tx_ring.add(slot)).status) };
        if status & STATUS_DD != 0 { return true; }
        n += 1;
        if n > 2_000_000 {
            log::warn!("[e1000] TX timeout (slot={slot})");
            return false;
        }
        core::hint::spin_loop();
    }
}

/// Receive one Ethernet frame into `buf`.
///
/// Returns the number of bytes received, or 0 if no frame is ready.
pub fn recv_frame(buf: &mut [u8]) -> usize {
    let mut guard = DEVICE.lock();
    let Some(nic) = guard.as_mut() else { return 0; };

    let slot = nic.rx_head;

    compiler_fence(Ordering::Acquire);

    // SAFETY: rx_ring[slot] is a valid DMA-allocated RxDesc.
    let status = unsafe { core::ptr::read_volatile(&(*nic.rx_ring.add(slot)).status) };
    if status & STATUS_DD == 0 {
        return 0; // no frame ready
    }

    // SAFETY: rx_ring[slot] is a valid DMA-allocated RxDesc.
    let pkt_len = unsafe {
        core::ptr::read_volatile(&(*nic.rx_ring.add(slot)).length) as usize
    };
    let copy_len = pkt_len.min(buf.len());

    // SAFETY: rx_bufs[slot] is a BUF_SIZE DMA buffer; pkt_len ≤ BUF_SIZE.
    unsafe {
        core::ptr::copy_nonoverlapping(nic.rx_bufs[slot], buf.as_mut_ptr(), copy_len);
    }

    // Reset descriptor status and replenish the buffer.
    // SAFETY: rx_ring[slot] is a valid DMA-allocated RxDesc.
    unsafe {
        let desc = &mut *nic.rx_ring.add(slot);
        core::ptr::write_volatile(&mut desc.status, 0);
        core::ptr::write_volatile(&mut desc.buf_addr, dma_phys(nic.rx_bufs[slot] as usize));
    }

    compiler_fence(Ordering::Release);

    // Return slot to hardware by advancing RDT.
    // SAFETY: bar0 is identity-mapped e1000 MMIO.
    unsafe { wr32(nic.bar0, RDT, slot as u32); }

    nic.rx_head = (slot + 1) % N_RX;

    copy_len
}

/// Force-unlock the DEVICE spinlock (called from the fault recovery path).
pub fn force_unlock_locks() {
    // SAFETY: called only from the kernel fault handler after a panic;
    // all other tasks are dead and won't try to re-acquire.
    unsafe { DEVICE.force_unlock(); }
}
