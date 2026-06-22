//! NVMe kernel block driver — implements `ViBlockDevice` behind polled completion.
//!
//! Initialisation sequence (NVMe 1.x spec §3.3):
//!   1. Map BAR0 MMIO (16 KiB).
//!   2. Reset controller (CC.EN=0), wait CSTS.RDY=0.
//!   3. Program AQA, ASQ, ACQ (64-entry admin queues).
//!   4. Enable (CC.EN=1), wait CSTS.RDY=1.
//!   5. Identify Controller (CNS=1) → check VWC flag for flush support.
//!   6. Identify Namespace 1 (CNS=0, NSID=1) → extract LBA count + format.
//!   7. Create I/O Completion Queue (admin opcode 0x05).
//!   8. Create I/O Submission Queue (admin opcode 0x01).
//!
//! DMA address translation (arch-specific):
//!   - riscv64 / aarch64: heap is identity-mapped → `phys = virt`.
//!   - x86_64: RAM is HHDM-mapped (not identity) → `phys = virt - phys_offset`.
//!     Programming the NVMe PRP entry with the raw VA would point the controller
//!     at the wrong physical page (silent corruption). Mirror the reasoning in
//!     `virtio_hal.rs:60-68`.
//!
//! Polled completion: the driver spins on the phase bit in the CQ entry.
//! MSI-X/IRQ-driven completion is a documented follow-up once the MSI-X cap
//! captured by `pcie_ecam::PciDevice.msix` is wired up to APIC/PLIC routes.
//!
//! IOMMU: deferred (marked as prerequisite for real G2 hardware in spec §8).
//! QEMU NVMe DMA is safe without IOMMU because the kernel is the only tenant.

use alloc::alloc::{alloc, dealloc, Layout};
use core::sync::atomic::{fence, Ordering};
use crate::sync::Spinlock;
use crate::task::drivers::pcie_ecam;
use api::block::ViBlockDevice;
use types::{ViError, ViResult};

// ── NVMe class / subclass / prog-if ──────────────────────────────────────────

const NVME_CLASS:   u8 = 0x01; // Mass Storage
const NVME_SUB:     u8 = 0x08; // NVM subsystem
const NVME_PROGIF:  u8 = 0x02; // NVM Express

// ── BAR0 register offsets (NVMe 1.x §3.1) ────────────────────────────────────

const REG_CAP:  usize = 0x00; // u64
const REG_VS:   usize = 0x08; // u32
const REG_CC:   usize = 0x14; // u32
const REG_CSTS: usize = 0x1C; // u32
const REG_AQA:  usize = 0x24; // u32
const REG_ASQ:  usize = 0x28; // u64
const REG_ACQ:  usize = 0x30; // u64
/// Doorbell base offset; stride = 4 << CAP.DSTRD per queue pair.
const REG_DB_BASE: usize = 0x1000;

// CC register bits.
const CC_EN:   u32 = 1 << 0;
const CC_CSS_NVM: u32 = 0; // CSS = 000 → NVM command set
const CC_MPS_4K: u32 = 0; // MPS = 0 → 4 KiB host pages (2^(12+0))
const CC_AMS_RR: u32 = 0; // AMS = 000 → round-robin
const CC_IOSQES: u32 = 6 << 16; // I/O SQ entry size = 2^6 = 64 bytes
const CC_IOCQES: u32 = 4 << 20; // I/O CQ entry size = 2^4 = 16 bytes

// CSTS bits.
const CSTS_RDY: u32 = 1 << 0;
const CSTS_CFS: u32 = 1 << 1;

// AQA: set admin SQ and CQ sizes to ADMIN_QUEUE_DEPTH - 1.
const ADMIN_QUEUE_DEPTH: u32 = 64;

// I/O queue depth (64 entries).
const IO_QUEUE_DEPTH: u16 = 64;

// Timeout: number of polling iterations before logging a warning.
// On real hardware a read should complete in microseconds; 1M iterations
// gives a generous wall-clock budget while avoiding a silent infinite spin.
const POLL_WARN_ITERS: u64 = 1_000_000;

// NVMe admin command opcodes.
const ADMIN_OPC_DELETE_SQ:   u8 = 0x00;
const ADMIN_OPC_CREATE_SQ:   u8 = 0x01;
const ADMIN_OPC_DELETE_CQ:   u8 = 0x04;
const ADMIN_OPC_CREATE_CQ:   u8 = 0x05;
const ADMIN_OPC_IDENTIFY:    u8 = 0x06;

// NVM command opcodes.
const NVM_OPC_FLUSH: u8 = 0x00;
const NVM_OPC_WRITE: u8 = 0x01;
const NVM_OPC_READ:  u8 = 0x02;

// Identify CNS values.
const CNS_IDENTIFY_NS:   u32 = 0; // identify namespace
const CNS_IDENTIFY_CTRL: u32 = 1; // identify controller

// ── Queue entry types (NVMe 1.x §4.2, §4.4) ──────────────────────────────────

/// NVMe Submission Queue Entry (64 bytes).
#[repr(C, align(64))]
#[derive(Clone, Copy, Default)]
struct SqEntry {
    cdw0:  u32, // opcode[7:0], fuse[9:8], PSDT[15:14], CID[31:16]
    nsid:  u32,
    cdw2:  u32,
    cdw3:  u32,
    mptr:  u64,
    prp1:  u64, // Physical Region Page 1
    prp2:  u64, // Physical Region Page 2 (0 for ≤4 KiB transfers)
    cdw10: u32,
    cdw11: u32,
    cdw12: u32,
    cdw13: u32,
    cdw14: u32,
    cdw15: u32,
}

/// NVMe Completion Queue Entry (16 bytes).
#[repr(C, align(16))]
#[derive(Clone, Copy, Default)]
struct CqEntry {
    dw0:   u32,
    dw1:   u32,
    sq_hd: u16, // SQ head pointer
    sq_id: u16, // SQ identifier
    cid:   u16, // Command identifier
    phase_status: u16, // phase bit[0], status[15:1]
}

impl CqEntry {
    #[inline]
    fn phase(&self) -> bool {
        self.phase_status & 1 != 0
    }
    #[inline]
    fn status_field(&self) -> u16 {
        self.phase_status >> 1
    }
}

// ── DMA helpers ───────────────────────────────────────────────────────────────

/// Translate a virtual address to its DMA physical address.
///
/// - riscv64 / aarch64: identity-mapped heap → phys == virt.
/// - x86_64: RAM mapped via HHDM → phys = virt - phys_offset().
///
/// Using the raw VA as a DMA address on x86_64 would silently corrupt memory
/// because the controller would DMA to `virt` as a physical address (pointing
/// into the HHDM window, not the actual physical frame).
#[inline]
fn dma_phys(virt: usize) -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        let offset = crate::memory::frame::phys_to_virt(0); // HHDM base
        // SAFETY: on x86_64 Limine maps all RAM at HHDM+phys, so phys = virt - HHDM.
        (virt - offset) as u64
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        virt as u64
    }
}

/// Allocate `n` contiguous 4 KiB pages, zeroed, for DMA use.
///
/// Returns (virt_ptr, phys_addr). Caller must free with `dma_free_pages`.
///
/// # Panics
/// Panics on OOM (mirrors `VirtioHal::dma_alloc` panic contract).
fn dma_alloc_pages(n: usize) -> (*mut u8, u64) {
    let size = n * 4096;
    let layout = Layout::from_size_align(size, 4096)
        .expect("NVMe DMA layout: pages>0 and 4096 is power-of-two");
    // SAFETY: layout is non-zero and properly aligned.
    let ptr = unsafe { alloc(layout) };
    if ptr.is_null() {
        panic!("[nvme] DMA alloc OOM: {} pages requested", n);
    }
    // SAFETY: ptr is non-null and points to `size` bytes.
    unsafe { core::ptr::write_bytes(ptr, 0, size); }
    let phys = dma_phys(ptr as usize);
    super::iommu::map_dma(phys, size);
    (ptr, phys)
}

/// Free DMA pages previously allocated with `dma_alloc_pages`.
///
/// # Safety
/// `ptr` must have been returned by `dma_alloc_pages(n)`.
unsafe fn dma_free_pages(ptr: *mut u8, n: usize) {
    let layout = Layout::from_size_align(n * 4096, 4096)
        .expect("NVMe DMA free layout: same as alloc");
    // SAFETY: caller guarantees ptr+n match the original alloc.
    unsafe { dealloc(ptr, layout); }
}

// ── BAR0 register accessors ───────────────────────────────────────────────────

/// Read a u64 register from the NVMe BAR0 MMIO region.
///
/// # Safety
/// `bar0` must be the identity-mapped NVMe BAR0 virtual base address.
#[inline]
unsafe fn read_reg64(bar0: usize, off: usize) -> u64 {
    // SAFETY: BAR0 is identity-mapped MMIO; volatile prevents optimisation.
    let ptr = (bar0 + off) as *const u64;
    unsafe { core::ptr::read_volatile(ptr) }
}

/// Read a u32 register from NVMe BAR0.
///
/// # Safety
/// `bar0` must be the identity-mapped NVMe BAR0 virtual base address.
#[inline]
unsafe fn read_reg32(bar0: usize, off: usize) -> u32 {
    // SAFETY: BAR0 is identity-mapped MMIO; volatile prevents optimisation.
    let ptr = (bar0 + off) as *const u32;
    unsafe { core::ptr::read_volatile(ptr) }
}

/// Write a u32 register to NVMe BAR0.
///
/// # Safety
/// `bar0` must be the identity-mapped NVMe BAR0 virtual base address.
#[inline]
unsafe fn write_reg32(bar0: usize, off: usize, val: u32) {
    // SAFETY: BAR0 is identity-mapped MMIO; volatile ensures write reaches HW.
    let ptr = (bar0 + off) as *mut u32;
    unsafe { core::ptr::write_volatile(ptr, val); }
}

/// Write a u64 register to NVMe BAR0.
///
/// # Safety
/// `bar0` must be the identity-mapped NVMe BAR0 virtual base address.
#[inline]
unsafe fn write_reg64(bar0: usize, off: usize, val: u64) {
    // SAFETY: BAR0 is identity-mapped MMIO; volatile ensures write reaches HW.
    let ptr = (bar0 + off) as *mut u64;
    unsafe { core::ptr::write_volatile(ptr, val); }
}

// ── Queue state ───────────────────────────────────────────────────────────────

struct Queue {
    /// Submission queue entries (physically contiguous DMA memory).
    sq:       *mut SqEntry,
    sq_phys:  u64,
    /// Completion queue entries (physically contiguous DMA memory).
    cq:       *mut CqEntry,
    cq_phys:  u64,
    /// Number of entries per queue (must be identical for SQ and CQ).
    depth:    u16,
    /// SQ tail pointer (wraps at depth).
    sq_tail:  u16,
    /// CQ head pointer (wraps at depth).
    cq_head:  u16,
    /// Expected phase bit for the next CQ entry.
    cq_phase: bool,
    /// Monotonically increasing command ID (wraps at u16::MAX).
    cid:      u16,
}

impl Queue {
    /// Allocate a new queue pair of `depth` entries.
    fn new(depth: u16) -> Self {
        let sq_pages = (depth as usize * core::mem::size_of::<SqEntry>()).div_ceil(4096);
        let cq_pages = (depth as usize * core::mem::size_of::<CqEntry>()).div_ceil(4096);
        let (sq, sq_phys) = dma_alloc_pages(sq_pages);
        let (cq, cq_phys) = dma_alloc_pages(cq_pages);
        Self {
            sq:       sq as *mut SqEntry,
            sq_phys,
            cq:       cq as *mut CqEntry,
            cq_phys,
            depth,
            sq_tail:  0,
            cq_head:  0,
            cq_phase: true, // initial phase = 1 after reset
            cid:      0,
        }
    }

    /// Free underlying DMA pages. Called from `Drop`.
    ///
    /// # Safety
    /// Must only be called once; `sq` and `cq` become dangling after this call.
    unsafe fn free(&mut self) {
        let sq_pages = (self.depth as usize * core::mem::size_of::<SqEntry>()).div_ceil(4096);
        let cq_pages = (self.depth as usize * core::mem::size_of::<CqEntry>()).div_ceil(4096);
        // SAFETY: ptrs were allocated by dma_alloc_pages with matching depths.
        unsafe { dma_free_pages(self.sq as *mut u8, sq_pages); }
        unsafe { dma_free_pages(self.cq as *mut u8, cq_pages); }
    }

    /// Allocate the next SQ slot. Returns slot index.
    fn next_cid(&mut self) -> u16 {
        self.cid = self.cid.wrapping_add(1);
        self.cid
    }
}

// ── NvmeController ───────────────────────────────────────────────────────────

/// Live NVMe controller state (stored behind a Spinlock).
pub struct NvmeController {
    /// Virtual base address of the NVMe BAR0 MMIO region (identity-mapped).
    bar0: usize,
    /// Admin queue (depth = ADMIN_QUEUE_DEPTH, QID = 0).
    admin: Queue,
    /// I/O queue pair (depth = IO_QUEUE_DEPTH, QID = 1).
    io:    Queue,
    /// Doorbell stride in bytes (4 << CAP.DSTRD).
    db_stride: usize,
    /// Total LBA count from Identify Namespace 1.
    pub n_sectors: u64,
    /// LBA size in bytes (expected 512).
    pub lba_bytes: u32,
    /// VWC (volatile write cache) supported → Flush command is meaningful.
    pub vwc: bool,
}

impl NvmeController {
    /// Initialise the NVMe controller mapped at `bar0`.
    ///
    /// Returns `Err(ViError::IO)` if the controller fails to reach RDY state or
    /// Identify fails. Does NOT panic — the caller falls through to VirtIO on error.
    ///
    /// # Safety
    /// `bar0` must be the identity-mapped virtual base of the NVMe BAR0 MMIO window.
    unsafe fn new(bar0: usize) -> ViResult<Self> {
        // 1. Read capabilities.
        // SAFETY: bar0 is identity-mapped BAR0.
        let cap = unsafe { read_reg64(bar0, REG_CAP) };
        let dstrd = ((cap >> 32) & 0xF) as usize; // CAP.DSTRD [35:32]
        let db_stride = 4 << dstrd;

        // 2. Reset controller: CC.EN=0, wait CSTS.RDY=0.
        // SAFETY: bar0 is identity-mapped BAR0.
        unsafe { write_reg32(bar0, REG_CC, 0); }
        let mut spin = 0u64;
        loop {
            // SAFETY: bar0 is identity-mapped BAR0.
            let csts = unsafe { read_reg32(bar0, REG_CSTS) };
            if csts & CSTS_RDY == 0 { break; }
            spin += 1;
            if spin > POLL_WARN_ITERS {
                log::warn!("[nvme] reset: CSTS.RDY stuck high after {} iters", spin);
                return Err(ViError::IO);
            }
            // SAFETY: sfence.vma / memory barrier to prevent reordering.
            fence(Ordering::SeqCst);
        }

        // 3. Allocate admin queues.
        let admin = Queue::new(ADMIN_QUEUE_DEPTH as u16);

        // 4. Program AQA, ASQ, ACQ.
        // AQA: ACQS[27:16] = depth-1, ASQS[11:0] = depth-1.
        let aqa = ((ADMIN_QUEUE_DEPTH - 1) << 16) | (ADMIN_QUEUE_DEPTH - 1);
        // SAFETY: bar0 is identity-mapped BAR0.
        unsafe { write_reg32(bar0, REG_AQA, aqa); }
        unsafe { write_reg64(bar0, REG_ASQ, admin.sq_phys); }
        unsafe { write_reg64(bar0, REG_ACQ, admin.cq_phys); }

        // 5. Enable controller: CC.EN=1, CSS=NVM, MPS=4K, AMS=RR, IOSQES/IOCQES.
        let cc = CC_EN | CC_CSS_NVM | CC_MPS_4K | CC_AMS_RR | CC_IOSQES | CC_IOCQES;
        // SAFETY: bar0 is identity-mapped BAR0.
        unsafe { write_reg32(bar0, REG_CC, cc); }

        // 6. Wait CSTS.RDY=1.
        let mut spin = 0u64;
        loop {
            // SAFETY: bar0 is identity-mapped BAR0.
            let csts = unsafe { read_reg32(bar0, REG_CSTS) };
            if csts & CSTS_CFS != 0 {
                log::error!("[nvme] controller fatal status (CFS) during enable");
                return Err(ViError::IO);
            }
            if csts & CSTS_RDY != 0 { break; }
            spin += 1;
            if spin > POLL_WARN_ITERS {
                log::warn!("[nvme] enable: CSTS.RDY never set after {} iters", spin);
                return Err(ViError::IO);
            }
            fence(Ordering::SeqCst);
        }

        let mut ctrl = NvmeController {
            bar0,
            admin,
            io: Queue::new(IO_QUEUE_DEPTH),
            db_stride,
            n_sectors: 0,
            lba_bytes: 0,
            vwc: false,
        };

        // 7. Identify Controller.
        let (id_buf, _) = dma_alloc_pages(1);
        let id_phys = dma_phys(id_buf as usize);
        let res = ctrl.submit_admin(ADMIN_OPC_IDENTIFY, 0, 0, id_phys, 0,
                                    CNS_IDENTIFY_CTRL, 0, 0, 0, 0, 0);
        if res.is_err() {
            // SAFETY: id_buf was allocated with dma_alloc_pages(1).
            unsafe { dma_free_pages(id_buf, 1); }
            return Err(ViError::IO);
        }
        // VWC flag at byte 525 of Identify Controller data structure.
        // SAFETY: id_buf points to a 4 KiB zeroed DMA page (index 525 is in range).
        let vwc_byte = unsafe { *id_buf.add(525) };
        ctrl.vwc = vwc_byte & 1 != 0;
        // SAFETY: id_buf was allocated with dma_alloc_pages(1).
        unsafe { dma_free_pages(id_buf, 1); }

        // 8. Identify Namespace 1.
        let (ns_buf, _) = dma_alloc_pages(1);
        let ns_phys = dma_phys(ns_buf as usize);
        let res = ctrl.submit_admin(ADMIN_OPC_IDENTIFY, 1, 0, ns_phys, 0,
                                    CNS_IDENTIFY_NS, 0, 0, 0, 0, 0);
        if res.is_err() {
            // SAFETY: ns_buf was allocated with dma_alloc_pages(1).
            unsafe { dma_free_pages(ns_buf, 1); }
            return Err(ViError::IO);
        }
        // NSZE (namespace size in LBAs) at bytes [7:0] of Identify Namespace.
        // SAFETY: ns_buf is a valid 4 KiB page; offset 0 is [7:0].
        let nsze = unsafe { core::ptr::read_unaligned(ns_buf as *const u64) };
        // FLBAS (formatted LBA size) at byte 26; lower 4 bits index into LBAF array.
        // SAFETY: ns_buf is a valid 4 KiB page; byte 26 is in range.
        let flbas = unsafe { *ns_buf.add(26) } & 0x0F;
        // LBAFs start at byte 128; each entry is 4 bytes; LBADS is at [23:16].
        let lbaf_off = 128 + (flbas as usize) * 4;
        // SAFETY: lbaf_off <= 128 + 15*4 = 188; ns_buf is 4096 bytes.
        let lbaf_dw = unsafe { core::ptr::read_unaligned(ns_buf.add(lbaf_off) as *const u32) };
        let lbads = (lbaf_dw >> 16) & 0xFF; // log2(sector size)
        let lba_size = 1u32 << lbads;
        // SAFETY: ns_buf was allocated with dma_alloc_pages(1).
        unsafe { dma_free_pages(ns_buf, 1); }

        if lba_size != 512 {
            log::warn!(
                "[nvme] namespace 1 LBA size is {} bytes (expected 512). \
                 4K LBA support is a documented follow-up; aborting NVMe init.",
                lba_size
            );
            return Err(ViError::IO);
        }

        ctrl.n_sectors = nsze;
        ctrl.lba_bytes = lba_size;

        log::info!(
            "[nvme] NVMe: initialized NSID=1 sectors={} lba={}",
            nsze, lba_size
        );

        // 9. Create I/O Completion Queue (QID=1, PC=1 contiguous).
        //    CDW10: QSIZE[27:16]=depth-1, QID[15:0]=1.
        //    CDW11: IEN[1]=0 (polling), PC[0]=1 (physically contiguous).
        let io_cq_phys = ctrl.io.cq_phys;
        ctrl.submit_admin(
            ADMIN_OPC_CREATE_CQ,
            0, 0,
            io_cq_phys, 0,
            ((IO_QUEUE_DEPTH as u32 - 1) << 16) | 1, // CDW10: QSIZE | QID
            0x1,  // CDW11: PC=1
            0, 0, 0, 0,
        )?;

        // 10. Create I/O Submission Queue (QID=1, CQ=1, PC=1).
        //    CDW10: QSIZE[27:16]=depth-1, QID[15:0]=1.
        //    CDW11: CQID[31:16]=1, PC[0]=1.
        let io_sq_phys = ctrl.io.sq_phys;
        ctrl.submit_admin(
            ADMIN_OPC_CREATE_SQ,
            0, 0,
            io_sq_phys, 0,
            ((IO_QUEUE_DEPTH as u32 - 1) << 16) | 1, // CDW10: QSIZE | QID
            (1u32 << 16) | 0x1, // CDW11: CQID=1 | PC=1
            0, 0, 0, 0,
        )?;

        Ok(ctrl)
    }

    /// Doorbell virtual address for a given queue ID and type.
    ///
    /// # Safety
    /// `bar0` must be the identity-mapped BAR0.
    #[inline]
    fn sq_doorbell_addr(&self, qid: u16) -> usize {
        self.bar0 + REG_DB_BASE + (qid as usize) * 2 * self.db_stride
    }

    #[inline]
    fn cq_doorbell_addr(&self, qid: u16) -> usize {
        self.bar0 + REG_DB_BASE + (qid as usize) * 2 * self.db_stride + self.db_stride
    }

    /// Ring the SQ tail doorbell for queue `qid`.
    ///
    /// # Safety
    /// Doorbell address must be in the identity-mapped BAR0 window.
    #[inline]
    unsafe fn ring_sq_tail(&self, qid: u16, tail: u16) {
        let db = self.sq_doorbell_addr(qid) as *mut u32;
        // SAFETY: doorbell is in identity-mapped BAR0; volatile write required.
        unsafe { core::ptr::write_volatile(db, tail as u32); }
    }

    /// Ring the CQ head doorbell for queue `qid`.
    ///
    /// # Safety
    /// Doorbell address must be in the identity-mapped BAR0 window.
    #[inline]
    unsafe fn ring_cq_head(&self, qid: u16, head: u16) {
        let db = self.cq_doorbell_addr(qid) as *mut u32;
        // SAFETY: doorbell is in identity-mapped BAR0; volatile write required.
        unsafe { core::ptr::write_volatile(db, head as u32); }
    }

    /// Submit one command to the admin queue (QID=0) and poll for completion.
    ///
    /// Returns `Err(ViError::IO)` on timeout or non-zero NVMe status.
    #[allow(clippy::too_many_arguments)]
    fn submit_admin(
        &mut self,
        opc:   u8,
        nsid:  u32,
        prp2:  u64,
        prp1:  u64,
        _mptr: u64,
        cdw10: u32,
        cdw11: u32,
        cdw12: u32,
        cdw13: u32,
        cdw14: u32,
        cdw15: u32,
    ) -> ViResult<()> {
        let cid = self.admin.next_cid();
        let depth = self.admin.depth as usize;
        let tail  = self.admin.sq_tail as usize;

        // Build the SQE.
        // SAFETY: sq is DMA memory allocated for `depth` SqEntry slots; tail < depth.
        let sqe = unsafe { &mut *self.admin.sq.add(tail) };
        *sqe = SqEntry::default();
        sqe.cdw0  = (opc as u32) | ((cid as u32) << 16);
        sqe.nsid  = nsid;
        sqe.prp1  = prp1;
        sqe.prp2  = prp2;
        sqe.cdw10 = cdw10;
        sqe.cdw11 = cdw11;
        sqe.cdw12 = cdw12;
        sqe.cdw13 = cdw13;
        sqe.cdw14 = cdw14;
        sqe.cdw15 = cdw15;

        // Advance tail.
        self.admin.sq_tail = ((tail + 1) % depth) as u16;

        // Memory fence before doorbell write.
        fence(Ordering::Release);

        // Ring SQ doorbell.
        // SAFETY: doorbell is in identity-mapped BAR0.
        unsafe { self.ring_sq_tail(0, self.admin.sq_tail); }

        // Poll CQ for the matching completion entry.
        let expected_phase = self.admin.cq_phase;
        let cq_head = self.admin.cq_head as usize;

        let mut iters = 0u64;
        loop {
            // SAFETY: cq is DMA memory; cq_head < depth.
            let cqe = unsafe { &*self.admin.cq.add(cq_head) };
            // Volatile read of phase_status to see controller writes.
            let phase_status = unsafe {
                core::ptr::read_volatile(&cqe.phase_status as *const u16)
            };
            let phase_bit = (phase_status & 1) != 0;
            if phase_bit == expected_phase {
                // Entry is valid. Decode status.
                let status = phase_status >> 1;
                // Advance CQ head.
                let new_head = (cq_head + 1) % depth;
                if new_head == 0 {
                    self.admin.cq_phase = !self.admin.cq_phase;
                }
                self.admin.cq_head = new_head as u16;
                // Ring CQ head doorbell.
                // SAFETY: doorbell is in identity-mapped BAR0.
                unsafe { self.ring_cq_head(0, self.admin.cq_head); }
                if status != 0 {
                    log::error!("[nvme] admin cmd opc={:#x} status={:#x}", opc, status);
                    return Err(ViError::IO);
                }
                return Ok(());
            }
            iters += 1;
            if iters == POLL_WARN_ITERS {
                log::warn!("[nvme] admin poll timeout after {} iters (opc={:#x})", iters, opc);
                return Err(ViError::IO);
            }
            fence(Ordering::Acquire);
        }
    }

    /// Submit one NVM I/O command (read or write) via the I/O queue (QID=1)
    /// and poll for completion.
    ///
    /// `prp1` must be the DMA physical address of a 512-byte-aligned buffer.
    fn submit_io(
        &mut self,
        opc:  u8,
        nsid: u32,
        lba:  u64,
        nlb:  u16, // 0-based: 0 = 1 block
        prp1: u64,
    ) -> ViResult<()> {
        let cid   = self.io.next_cid();
        let depth = self.io.depth as usize;
        let tail  = self.io.sq_tail as usize;

        // SAFETY: io.sq is DMA memory for `depth` SqEntry slots; tail < depth.
        let sqe = unsafe { &mut *self.io.sq.add(tail) };
        *sqe = SqEntry::default();
        sqe.cdw0  = (opc as u32) | ((cid as u32) << 16);
        sqe.nsid  = nsid;
        sqe.prp1  = prp1;
        sqe.prp2  = 0; // single-page transfer (≤4 KiB)
        sqe.cdw10 = lba as u32;          // SLBA[31:0]
        sqe.cdw11 = (lba >> 32) as u32;  // SLBA[63:32]
        sqe.cdw12 = nlb as u32;          // NLB (0-based, 0 = 1 sector)

        self.io.sq_tail = ((tail + 1) % depth) as u16;

        fence(Ordering::Release);
        // SAFETY: doorbell is in identity-mapped BAR0.
        unsafe { self.ring_sq_tail(1, self.io.sq_tail); }

        // Poll I/O CQ.
        let expected_phase = self.io.cq_phase;
        let cq_head = self.io.cq_head as usize;

        let mut iters = 0u64;
        loop {
            // SAFETY: io.cq is DMA memory; cq_head < depth.
            let phase_status = unsafe {
                let cqe = &*self.io.cq.add(cq_head);
                core::ptr::read_volatile(&cqe.phase_status as *const u16)
            };
            let phase_bit = (phase_status & 1) != 0;
            if phase_bit == expected_phase {
                let status = phase_status >> 1;
                let new_head = (cq_head + 1) % depth;
                if new_head == 0 {
                    self.io.cq_phase = !self.io.cq_phase;
                }
                self.io.cq_head = new_head as u16;
                // SAFETY: doorbell is in identity-mapped BAR0.
                unsafe { self.ring_cq_head(1, self.io.cq_head); }
                if status != 0 {
                    log::error!("[nvme] I/O cmd opc={:#x} lba={} status={:#x}", opc, lba, status);
                    return Err(ViError::IO);
                }
                return Ok(());
            }
            iters += 1;
            if iters == POLL_WARN_ITERS {
                log::warn!("[nvme] I/O poll timeout after {} iters", iters);
                return Err(ViError::IO);
            }
            fence(Ordering::Acquire);
        }
    }
}

impl Drop for NvmeController {
    /// Disable the NVMe controller and free queue DMA memory.
    ///
    /// This is a best-effort teardown — it mirrors `virtio_blk.rs` cleanup.
    fn drop(&mut self) {
        // Delete I/O submission queue (QID=1).
        let _ = self.submit_admin(ADMIN_OPC_DELETE_SQ, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0);
        // Delete I/O completion queue (QID=1).
        let _ = self.submit_admin(ADMIN_OPC_DELETE_CQ, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0);

        // Disable controller (CC.EN=0).
        // SAFETY: bar0 is the identity-mapped NVMe BAR0; CC is a valid register.
        unsafe { write_reg32(self.bar0, REG_CC, 0); }

        // Free DMA pages.
        // SAFETY: admin/io queue ptrs were allocated with dma_alloc_pages;
        // Drop is called at most once per NvmeController.
        unsafe { self.admin.free(); }
        unsafe { self.io.free(); }

        log::info!("[nvme] controller disabled and DMA memory freed");
    }
}

// SAFETY: NvmeController contains raw pointers into DMA memory that are only
// accessed under the global NVME Spinlock, making single-threaded accesses safe.
unsafe impl Send for NvmeController {}
unsafe impl Sync for NvmeController {}

// ── Global controller slot ────────────────────────────────────────────────────

/// Global NVMe controller instance (kernel-internal; kernel-only access).
static NVME: Spinlock<Option<NvmeController>> = Spinlock::new(None);

/// Force-release this module's lock during fault teardown.
///
/// # Safety
/// Single-hart; called only from the fault/panic path with interrupts disabled.
pub unsafe fn force_unlock_locks() {
    // SAFETY: caller guarantees single-hart abort context.
    unsafe { NVME.force_unlock(); }
}

/// Returns `true` when an NVMe controller was successfully initialised.
pub fn is_present() -> bool {
    NVME.lock().is_some()
}

/// Initialise the NVMe driver.
///
/// Called from `drivers::init()` on architectures that have PCIe. Scans the
/// ECAM device list for an NVMe endpoint, maps BAR0, and runs the init sequence.
/// Falls through silently if no NVMe device is found (VirtIO remains the
/// block device).
pub fn init_driver() {
    let dev = match pcie_ecam::find_class(NVME_CLASS, NVME_SUB, NVME_PROGIF) {
        Some(d) => d,
        None => {
            log::info!("[nvme] no NVMe device found on PCIe bus");
            return;
        }
    };

    log::info!(
        "[nvme] found NVMe device {:02x}:{:02x}.{} bar0={:#x}",
        dev.bdf.0, dev.bdf.1, dev.bdf.2,
        dev.bars[0].base_addr()
    );

    // BAR0 for NVMe is always a 64-bit MMIO BAR.
    let bar0_phys = dev.bars[0].base_addr() as usize;
    if bar0_phys == 0 {
        log::warn!("[nvme] BAR0 is 0 — NVMe MMIO not configured by firmware");
        return;
    }

    // On x86_64, the BAR0 physical address is below the HHDM window (MMIO),
    // so it is identity-mapped (VA == PA) by the MMIO pages we add to the PML4.
    // On riscv64/aarch64 the MMIO is also identity-mapped by init_kernel_paging.
    // bar0_virt == bar0_phys in both cases for MMIO regions.
    let bar0_virt = bar0_phys;

    // SAFETY: bar0_virt is the identity-mapped BAR0 of the discovered NVMe device.
    // The MMIO window is mapped in `init_kernel_paging*` before this function.
    match unsafe { NvmeController::new(bar0_virt) } {
        Ok(ctrl) => {
            *NVME.lock() = Some(ctrl);
            log::info!("[nvme] driver ready — NVMe block device active");
        }
        Err(e) => {
            log::warn!("[nvme] controller init failed ({:?}); falling back to VirtIO", e);
        }
    }
}

// ── ZST proxy + ViBlockDevice impl ───────────────────────────────────────────

/// Zero-sized type used as the `&'static dyn ViBlockDevice` handle.
///
/// All state lives in the global `NVME` Spinlock; this ZST just provides the
/// vtable entry point, mirroring `viVirtIOBlk` in `virtio_blk.rs`.
#[allow(non_camel_case_types)]
pub struct NvmeBlk;

impl ViBlockDevice for NvmeBlk {
    fn read_sector(&self, sector: u64, buf: &mut [u8]) -> ViResult<()> {
        if buf.len() < 512 {
            return Err(ViError::InvalidArgument);
        }

        // Allocate a 4 KiB-aligned DMA bounce buffer for this sector read.
        // Using a bounce buffer avoids alignment constraints on the caller's `buf`
        // and is safe across the polled completion because the buffer outlives
        // the command (no async handoff, no lifetime issue).
        let (dma, phys) = dma_alloc_pages(1);

        let mut guard = NVME.lock();
        let Some(ctrl) = guard.as_mut() else {
            // SAFETY: dma was allocated with dma_alloc_pages(1).
            unsafe { dma_free_pages(dma, 1); }
            return Err(ViError::NotFound);
        };

        let result = ctrl.submit_io(NVM_OPC_READ, 1, sector, 0, phys);

        if result.is_ok() {
            // Copy the DMA buffer into the caller's slice.
            // SAFETY: dma points to a valid 4 KiB page; we copy only 512 bytes.
            let src = unsafe { core::slice::from_raw_parts(dma, 512) };
            buf[..512].copy_from_slice(src);
        }

        // SAFETY: dma was allocated with dma_alloc_pages(1).
        unsafe { dma_free_pages(dma, 1); }
        result
    }

    fn write_sector(&self, sector: u64, buf: &[u8]) -> ViResult<()> {
        if buf.len() < 512 {
            return Err(ViError::InvalidArgument);
        }

        let (dma, phys) = dma_alloc_pages(1);
        // Copy caller's data into the DMA bounce buffer.
        // SAFETY: dma points to a valid 4 KiB page; we write only 512 bytes.
        unsafe { core::ptr::copy_nonoverlapping(buf.as_ptr(), dma, 512); }

        let mut guard = NVME.lock();
        let Some(ctrl) = guard.as_mut() else {
            // SAFETY: dma was allocated with dma_alloc_pages(1).
            unsafe { dma_free_pages(dma, 1); }
            return Err(ViError::NotFound);
        };

        let result = ctrl.submit_io(NVM_OPC_WRITE, 1, sector, 0, phys);
        // SAFETY: dma was allocated with dma_alloc_pages(1).
        unsafe { dma_free_pages(dma, 1); }
        result
    }

    fn sector_count(&self) -> u64 {
        NVME.lock()
            .as_ref()
            .map(|c| c.n_sectors)
            .unwrap_or(0)
    }

    fn sector_size(&self) -> usize {
        NVME.lock()
            .as_ref()
            .map(|c| c.lba_bytes as usize)
            .unwrap_or(512)
    }

    fn flush(&self) -> ViResult<()> {
        let mut guard = NVME.lock();
        let Some(ctrl) = guard.as_mut() else {
            return Err(ViError::NotFound);
        };
        if !ctrl.vwc {
            // No volatile write cache — flush is a no-op.
            return Ok(());
        }
        ctrl.submit_io(NVM_OPC_FLUSH, 1, 0, 0, 0)
    }
}
