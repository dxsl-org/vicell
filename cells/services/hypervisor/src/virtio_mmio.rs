//! virtio-mmio Version=2 register-block emulator for one device slot.
//!
//! Handshake: ACK(1)→DRIVER(2)→feature exchange→FEATURES_OK(8)→queue setup→DRIVER_OK(4).
//! VERSION_1 (bit 32 = DriverFeatures high-word bit 0) is mandatory; rejected otherwise.

/// Maximum queues per device.
pub const MAX_QUEUES: usize = 2;
const QUEUE_SIZE_MAX: u16 = 256;
// VIRTIO_F_VERSION_1 sits in feature high-word bit 0 (bit 32 of the 64-bit field).
const VIRTIO_F_VERSION_1_HI: u32 = 1;

/// Per-queue GPA layout written by the driver during initialization.
#[derive(Default, Clone, Copy)]
pub struct QueueCfg {
    pub num:       u16,
    pub ready:     bool,
    pub desc_gpa:  u64,
    pub avail_gpa: u64,
    pub used_gpa:  u64,
}

/// Device model contract.
pub trait VirtioDevice {
    fn device_id(&self) -> u32;
    fn device_features_lo(&self) -> u32 { 0 }
    fn device_features_hi(&self) -> u32 { VIRTIO_F_VERSION_1_HI }
    /// Guest rang QueueNotify for queue `q` with the confirmed queue config.
    fn notify(&mut self, q: usize, qcfg: &QueueCfg, vm_id: usize, vcpu_id: usize);
    fn config_read(&self, _offset: usize) -> u32 { 0 }
}

/// Register state for one virtio-mmio slot.
#[derive(Default)]
pub struct VirtioMmio {
    status:          u32,
    feat_sel:        u32,
    drv_feat_sel:    u32,
    drv_feat_lo:     u32,
    drv_feat_hi:     u32,
    queue_sel:       usize,
    queues:          [QueueCfg; MAX_QUEUES],
    pub intr_status: u32,
}

impl VirtioMmio {
    pub fn mmio_read(&self, offset: u64, dev: &dyn VirtioDevice) -> u64 {
        let q = self.queue_sel;
        match offset {
            0x000 => 0x7472_6976,                          // Magic "virt"
            0x004 => 2,                                    // Version=2 (modern)
            0x008 => dev.device_id() as u64,
            0x00c => 0xFFFF_FFFF,                          // VendorID
            0x010 => if self.feat_sel == 0 {
                dev.device_features_lo() as u64
            } else {
                dev.device_features_hi() as u64
            },
            0x034 => QUEUE_SIZE_MAX as u64,                // QueueNumMax
            0x038 => if q < MAX_QUEUES { self.queues[q].num as u64 } else { 0 },
            0x044 => if q < MAX_QUEUES && self.queues[q].ready { 1 } else { 0 },
            0x060 => self.intr_status as u64,
            0x070 => self.status as u64,
            o if o >= 0x100 => dev.config_read((o - 0x100) as usize) as u64,
            _ => 0,
        }
    }

    pub fn mmio_write(
        &mut self,
        offset: u64,
        val: u32,
        dev: &mut dyn VirtioDevice,
        vm_id: usize,
        vcpu_id: usize,
    ) {
        let q = self.queue_sel;
        match offset {
            0x014 => self.feat_sel = val,
            0x020 => if self.drv_feat_sel == 0 { self.drv_feat_lo = val; } else { self.drv_feat_hi = val; },
            0x024 => self.drv_feat_sel = val,
            0x030 => { if (val as usize) < MAX_QUEUES { self.queue_sel = val as usize; } }
            0x038 => { if q < MAX_QUEUES { self.queues[q].num = val.min(QUEUE_SIZE_MAX as u32) as u16; } }
            0x044 => { if q < MAX_QUEUES { self.queues[q].ready = val == 1; } }
            0x050 => {
                // QueueNotify: val = queue index signalled by the driver.
                let nq = val as usize;
                if nq < MAX_QUEUES && self.queues[nq].ready && self.queues[nq].num > 0 {
                    let qcfg = self.queues[nq];
                    dev.notify(nq, &qcfg, vm_id, vcpu_id);
                    self.intr_status |= 1; // used-buffer notification
                }
            }
            0x064 => self.intr_status &= !val,             // InterruptACK
            0x070 => {
                if val == 0 { *self = VirtioMmio::default(); return; } // device reset
                if val & 0x8 != 0 && self.drv_feat_hi & VIRTIO_F_VERSION_1_HI == 0 {
                    // Guest did not negotiate VERSION_1; signal NEEDS_RESET.
                    self.status |= 0x40;
                    return;
                }
                self.status = val;
                if val & 0x80 != 0 { self.status |= 0x40; } // FAILED → NEEDS_RESET
            }
            0x080 => { if q < MAX_QUEUES { set_lo(&mut self.queues[q].desc_gpa,  val); } }
            0x084 => { if q < MAX_QUEUES { set_hi(&mut self.queues[q].desc_gpa,  val); } }
            0x090 => { if q < MAX_QUEUES { set_lo(&mut self.queues[q].avail_gpa, val); } }
            0x094 => { if q < MAX_QUEUES { set_hi(&mut self.queues[q].avail_gpa, val); } }
            0x0a0 => { if q < MAX_QUEUES { set_lo(&mut self.queues[q].used_gpa,  val); } }
            0x0a4 => { if q < MAX_QUEUES { set_hi(&mut self.queues[q].used_gpa,  val); } }
            _ => {}
        }
    }

    /// Return a copy of the queue configuration for queue `q`.
    pub fn queue_cfg(&self, q: usize) -> QueueCfg {
        if q < MAX_QUEUES { self.queues[q] } else { QueueCfg::default() }
    }
}

#[inline] fn set_lo(v: &mut u64, lo: u32) { *v = (*v & 0xFFFF_FFFF_0000_0000) | lo as u64; }
#[inline] fn set_hi(v: &mut u64, hi: u32) { *v = (*v & 0x0000_0000_FFFF_FFFF) | ((hi as u64) << 32); }

/// virtio-mmio region: base 0x0a000000, stride 0x200, 32 slots.
pub const VIRTIO_MMIO_BASE:   u64 = 0x0a00_0000;
pub const VIRTIO_MMIO_STRIDE: u64 = 0x200;
pub const VIRTIO_MMIO_SLOTS:  u64 = 32;

pub fn owns(ipa: u64) -> bool {
    ipa >= VIRTIO_MMIO_BASE && ipa < VIRTIO_MMIO_BASE + VIRTIO_MMIO_SLOTS * VIRTIO_MMIO_STRIDE
}

pub fn slot_and_offset(ipa: u64) -> (usize, u64) {
    let rel = ipa - VIRTIO_MMIO_BASE;
    ((rel / VIRTIO_MMIO_STRIDE) as usize, rel % VIRTIO_MMIO_STRIDE)
}
