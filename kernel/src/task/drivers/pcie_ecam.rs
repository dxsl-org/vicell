//! PCIe ECAM (Enhanced Configuration Access Mechanism) walker.
//!
//! Scans bus 0, devices 0-31, probing functions per the header-type multi-function
//! bit. Exposes a kernel-internal snapshot of discovered PCI devices.
//!
//! ECAM bases per machine (QEMU defaults — hardcoded for v1; ACPI MCFG parse
//! and DTB `pci-host-ecam-generic` reg lookup are documented follow-ups):
//!   x86_64 q35  : 0xB000_0000
//!   RISC-V virt : 0x3000_0000
//!   ARM64 virt  : 0x3F00_0000
//!
//! Only bus 0 is mapped (1 MiB). Devices on buses > 0 require extending the
//! MMIO window — documented follow-up (no QEMU NVMe lands on bus > 0).
//!
//! Call ordering: `platform::init()` → `init_kernel_paging*()` → `pcie_ecam::init()`.

use alloc::vec::Vec;
use crate::sync::Spinlock;

// ── ECAM base addresses (QEMU machine defaults) ───────────────────────────────

/// PCIe ECAM config-space base for x86_64 q35.
/// Source: QEMU q35 machine (pcie.0 bus at 0xB000_0000).
/// Follow-up: parse ACPI MCFG table to support non-q35 x86 boards.
pub const ECAM_BASE_X86: usize = 0xB000_0000;

/// PCIe ECAM config-space base for RISC-V virt gpex.
/// Source: QEMU virt machine DTS — `pci@30000000`.
pub const ECAM_BASE_RISCV: usize = 0x3000_0000;

/// PCIe ECAM config-space base for ARM64 virt gpex.
/// Source: QEMU virt machine DTS — `pcie@3f000000`.
pub const ECAM_BASE_AARCH64: usize = 0x3F00_0000;

/// Bus 0 ECAM window size (1 MiB = 32 devices × 8 functions × 4 KiB).
pub const ECAM_BUS0_SIZE: usize = 0x10_0000; // 1 MiB

// ── Config space offsets (PCI 3.0 type-0 header) ─────────────────────────────

const CFG_VENDOR_ID:   usize = 0x00;
const CFG_DEVICE_ID:   usize = 0x02;
const CFG_COMMAND:     usize = 0x04;
const CFG_CLASS_PROG:  usize = 0x09; // Prog IF
const CFG_SUBCLASS:    usize = 0x0A;
const CFG_CLASS_CODE:  usize = 0x0B;
const CFG_HEADER_TYPE: usize = 0x0E;
const CFG_BAR0:        usize = 0x10;
const CFG_CAP_PTR:     usize = 0x34;
const CFG_STATUS:      usize = 0x06;

// Command register bits.
const CMD_MEM_SPACE: u16 = 1 << 1;

// Capability IDs.
const CAP_ID_PM:   u8 = 0x01;
const CAP_ID_MSIX: u8 = 0x11;

// ── Public types ──────────────────────────────────────────────────────────────

/// A Base Address Register decoded from PCI config space.
#[derive(Clone, Copy, Debug)]
pub enum Bar {
    /// 32-bit MMIO BAR: base address and probed size.
    Memory32 { addr: u32, size: u32 },
    /// 64-bit MMIO BAR: base address (64-bit) and probed size. Consumes two slots.
    Memory64 { addr: u64, size: u64 },
    /// I/O port BAR (not mapped by this driver; skipped).
    Io,
    /// Unused / empty BAR slot.
    None,
}

impl Bar {
    /// Physical base address of this BAR, or 0 for I/O / empty.
    pub fn base_addr(&self) -> u64 {
        match self {
            Bar::Memory32 { addr, .. } => *addr as u64,
            Bar::Memory64 { addr, .. } => *addr,
            _ => 0,
        }
    }
}

/// MSI-X capability record captured during capability-list walk.
#[derive(Clone, Copy, Debug)]
pub struct MsixCap {
    /// Offset of the MSI-X capability structure in config space.
    pub cap_offset: u8,
    /// MSI-X Message Control register (table size - 1 in bits [10:0]).
    pub msg_ctrl: u16,
}

/// PM (Power Management) capability record.
#[derive(Clone, Copy, Debug)]
pub struct PmCap {
    /// Offset of the PM capability structure in config space.
    pub cap_offset: u8,
}

/// A discovered PCI function (device + function).
#[derive(Clone, Debug)]
pub struct PciDevice {
    /// (bus, device, function).
    pub bdf:      (u8, u8, u8),
    pub vendor_id: u16,
    pub device_id: u16,
    /// Class code (offset 0x0B).
    pub class:    u8,
    /// Subclass (offset 0x0A).
    pub subclass: u8,
    /// Programming interface (offset 0x09).
    pub prog_if:  u8,
    /// Up to 6 BARs (64-bit BARs consume two slots; slot N+1 is `Bar::None`).
    pub bars:     [Bar; 6],
    /// MSI-X capability, if present in capability list.
    pub msix:     Option<MsixCap>,
    /// PM capability, if present.
    pub pm:       Option<PmCap>,
}

// ── Global device list ────────────────────────────────────────────────────────

static PCI_DEVICES: Spinlock<Vec<PciDevice>> = Spinlock::new(Vec::new());

// ── ECAM MMIO accessors ───────────────────────────────────────────────────────

/// Compute MMIO address for a BDF + register offset.
///
/// ECAM formula: base + (bus << 20) + (device << 15) + (function << 12) + offset.
#[inline(always)]
fn config_addr(ecam_base: usize, bus: u8, dev: u8, fun: u8, off: usize) -> *mut u8 {
    let addr = ecam_base
        + ((bus  as usize) << 20)
        + ((dev  as usize) << 15)
        + ((fun  as usize) << 12)
        + off;
    addr as *mut u8
}

/// Read a u32 from PCI config space via ECAM MMIO.
///
/// # Safety
/// `config_addr(ecam_base, bus, dev, fun, off)` must point into the identity-
/// mapped ECAM window established by `init_kernel_paging*()`.  `off` must be
/// 4-byte-aligned.
unsafe fn read32(ecam_base: usize, bus: u8, dev: u8, fun: u8, off: usize) -> u32 {
    // SAFETY: caller guarantees the ECAM window is identity-mapped.
    let ptr = config_addr(ecam_base, bus, dev, fun, off) as *const u32;
    // SAFETY: volatile prevents the compiler from optimising away hardware reads.
    unsafe { core::ptr::read_volatile(ptr) }
}

/// Read a u16 from PCI config space via ECAM MMIO.
///
/// # Safety
/// Same contract as `read32`; `off` must be 2-byte-aligned.
unsafe fn read16(ecam_base: usize, bus: u8, dev: u8, fun: u8, off: usize) -> u16 {
    // SAFETY: caller guarantees the ECAM window is identity-mapped.
    let ptr = config_addr(ecam_base, bus, dev, fun, off) as *const u16;
    // SAFETY: volatile prevents optimisation of hardware register reads.
    unsafe { core::ptr::read_volatile(ptr) }
}

/// Read a u8 from PCI config space via ECAM MMIO.
///
/// # Safety
/// Same contract as `read32`.
unsafe fn read8(ecam_base: usize, bus: u8, dev: u8, fun: u8, off: usize) -> u8 {
    // SAFETY: caller guarantees the ECAM window is identity-mapped.
    let ptr = config_addr(ecam_base, bus, dev, fun, off);
    // SAFETY: volatile prevents optimisation of hardware register reads.
    unsafe { core::ptr::read_volatile(ptr) }
}

/// Write a u32 to PCI config space via ECAM MMIO.
///
/// # Safety
/// Same contract as `read32`; used only for BAR size-probe writes which are
/// bounded to the currently scanned function and immediately restored.
unsafe fn write32(ecam_base: usize, bus: u8, dev: u8, fun: u8, off: usize, val: u32) {
    // SAFETY: caller guarantees the ECAM window is identity-mapped.
    let ptr = config_addr(ecam_base, bus, dev, fun, off) as *mut u32;
    // SAFETY: volatile ensures the write reaches the MMIO register.
    unsafe { core::ptr::write_volatile(ptr, val); }
}

/// Write a u16 to PCI config space.
///
/// # Safety
/// Same contract as `write32`.
unsafe fn write16(ecam_base: usize, bus: u8, dev: u8, fun: u8, off: usize, val: u16) {
    // SAFETY: caller guarantees the ECAM window is identity-mapped.
    let ptr = config_addr(ecam_base, bus, dev, fun, off) as *mut u16;
    // SAFETY: volatile ensures the write reaches the MMIO register.
    unsafe { core::ptr::write_volatile(ptr, val); }
}

// ── BAR decode ────────────────────────────────────────────────────────────────

/// Decode all BARs for a type-0 function.
///
/// The write-all-ones / read-back size-probe modifies the command register to
/// disable memory decode before each probe and restores it after, preventing
/// the device from responding to MMIO accesses during the transient probe window.
///
/// # Safety
/// `ecam_base` must be the identity-mapped ECAM window base.
unsafe fn decode_bars(
    ecam_base: usize,
    bus: u8,
    dev: u8,
    fun: u8,
) -> [Bar; 6] {
    let mut bars = [Bar::None; 6];
    let mut i = 0usize;
    while i < 6 {
        let off = CFG_BAR0 + i * 4;
        // SAFETY: ECAM window is identity-mapped; off is within type-0 header.
        let raw = unsafe { read32(ecam_base, bus, dev, fun, off) };
        if raw & 1 == 1 {
            // I/O BAR — skip.
            bars[i] = Bar::Io;
            i += 1;
            continue;
        }
        let bar_type = (raw >> 1) & 0x3; // bits [2:1]
        let addr32 = raw & !0xF;

        if bar_type == 0x2 {
            // 64-bit MMIO: base spans this slot + next.
            if i + 1 >= 6 {
                bars[i] = Bar::None;
                i += 1;
                continue;
            }
            let off_hi = CFG_BAR0 + (i + 1) * 4;
            // SAFETY: ECAM window is identity-mapped.
            let raw_hi = unsafe { read32(ecam_base, bus, dev, fun, off_hi) };
            let addr64 = (addr32 as u64) | ((raw_hi as u64) << 32);
            let size64 = unsafe { probe_bar_size64(ecam_base, bus, dev, fun, i) };
            bars[i]     = Bar::Memory64 { addr: addr64, size: size64 };
            bars[i + 1] = Bar::None; // second slot consumed
            i += 2;
        } else {
            // 32-bit MMIO.
            let size32 = unsafe { probe_bar_size32(ecam_base, bus, dev, fun, i) };
            bars[i] = Bar::Memory32 { addr: addr32, size: size32 };
            i += 1;
        }
    }
    bars
}

/// Probe size of a 32-bit MMIO BAR via the write-all-ones / read-back method.
///
/// Saves/restores both the command register (disabling memory decode during the
/// probe) and the BAR value so the device is left in its original state.
///
/// # Safety
/// `ecam_base` must be the identity-mapped ECAM window base.
unsafe fn probe_bar_size32(
    ecam_base: usize,
    bus: u8,
    dev: u8,
    fun: u8,
    bar_idx: usize,
) -> u32 {
    let off = CFG_BAR0 + bar_idx * 4;
    // Save original values.
    // SAFETY: ECAM window is identity-mapped.
    let orig_cmd = unsafe { read16(ecam_base, bus, dev, fun, CFG_COMMAND) };
    // SAFETY: ECAM window is identity-mapped.
    let orig_bar = unsafe { read32(ecam_base, bus, dev, fun, off) };

    // Disable memory decode before writing all-ones to BAR.
    // This prevents the device from claiming MMIO during the probe window.
    // SAFETY: writing command register disables decode only for this function.
    unsafe { write16(ecam_base, bus, dev, fun, CFG_COMMAND, orig_cmd & !CMD_MEM_SPACE); }

    // Write all-ones to BAR.
    // SAFETY: ECAM window is identity-mapped; BAR write is bounded to this function.
    unsafe { write32(ecam_base, bus, dev, fun, off, 0xFFFF_FFFF); }
    // SAFETY: ECAM window is identity-mapped.
    let readback = unsafe { read32(ecam_base, bus, dev, fun, off) };

    // Restore BAR and command register.
    // SAFETY: ECAM window is identity-mapped.
    unsafe { write32(ecam_base, bus, dev, fun, off, orig_bar); }
    unsafe { write16(ecam_base, bus, dev, fun, CFG_COMMAND, orig_cmd); }

    // Size = ~(readback & mask) + 1 where mask = 0xFFFFFFF0 (clear lower 4 bits).
    let mask = readback & 0xFFFF_FFF0;
    if mask == 0 { return 0; }
    (!mask).wrapping_add(1)
}

/// Probe size of a 64-bit MMIO BAR (both low and high dwords).
///
/// # Safety
/// `ecam_base` must be the identity-mapped ECAM window base.
unsafe fn probe_bar_size64(
    ecam_base: usize,
    bus: u8,
    dev: u8,
    fun: u8,
    bar_idx: usize,
) -> u64 {
    let off_lo = CFG_BAR0 + bar_idx * 4;
    let off_hi = CFG_BAR0 + (bar_idx + 1) * 4;

    // SAFETY: ECAM window is identity-mapped.
    let orig_cmd = unsafe { read16(ecam_base, bus, dev, fun, CFG_COMMAND) };
    // SAFETY: ECAM window is identity-mapped.
    let orig_lo  = unsafe { read32(ecam_base, bus, dev, fun, off_lo) };
    // SAFETY: ECAM window is identity-mapped.
    let orig_hi  = unsafe { read32(ecam_base, bus, dev, fun, off_hi) };

    // Disable memory decode during probe.
    // SAFETY: ECAM window is identity-mapped.
    unsafe { write16(ecam_base, bus, dev, fun, CFG_COMMAND, orig_cmd & !CMD_MEM_SPACE); }

    // Write all-ones to both halves.
    // SAFETY: ECAM window is identity-mapped.
    unsafe { write32(ecam_base, bus, dev, fun, off_lo, 0xFFFF_FFFF); }
    unsafe { write32(ecam_base, bus, dev, fun, off_hi, 0xFFFF_FFFF); }

    // SAFETY: ECAM window is identity-mapped.
    let rb_lo = unsafe { read32(ecam_base, bus, dev, fun, off_lo) };
    // SAFETY: ECAM window is identity-mapped.
    let rb_hi = unsafe { read32(ecam_base, bus, dev, fun, off_hi) };

    // Restore.
    // SAFETY: ECAM window is identity-mapped.
    unsafe { write32(ecam_base, bus, dev, fun, off_lo, orig_lo); }
    unsafe { write32(ecam_base, bus, dev, fun, off_hi, orig_hi); }
    unsafe { write16(ecam_base, bus, dev, fun, CFG_COMMAND, orig_cmd); }

    let mask64 = ((rb_hi as u64) << 32) | ((rb_lo & 0xFFFF_FFF0) as u64);
    if mask64 == 0 { return 0; }
    (!mask64).wrapping_add(1)
}

// ── Capability list walk ──────────────────────────────────────────────────────

/// Walk the capability list for a type-0 function; return MSI-X and PM caps.
///
/// # Safety
/// `ecam_base` must be the identity-mapped ECAM window base.
unsafe fn walk_caps(
    ecam_base: usize,
    bus: u8,
    dev: u8,
    fun: u8,
) -> (Option<MsixCap>, Option<PmCap>) {
    let mut msix = None;
    let mut pm   = None;

    // Capability list is present only when status bit 4 is set.
    // SAFETY: ECAM window is identity-mapped.
    let status = unsafe { read16(ecam_base, bus, dev, fun, CFG_STATUS) };
    if status & (1 << 4) == 0 {
        return (None, None);
    }

    // SAFETY: ECAM window is identity-mapped.
    let mut cap_ptr = unsafe { read8(ecam_base, bus, dev, fun, CFG_CAP_PTR) } & 0xFC;
    let mut budget  = 64u8; // guard against malformed circular lists

    while cap_ptr != 0 && budget > 0 {
        budget -= 1;
        // SAFETY: ECAM window is identity-mapped.
        let cap_id = unsafe { read8(ecam_base, bus, dev, fun, cap_ptr as usize) };
        // SAFETY: ECAM window is identity-mapped.
        let next   = unsafe { read8(ecam_base, bus, dev, fun, cap_ptr as usize + 1) } & 0xFC;

        match cap_id {
            CAP_ID_MSIX => {
                // SAFETY: ECAM window is identity-mapped.
                let msg_ctrl = unsafe {
                    read16(ecam_base, bus, dev, fun, cap_ptr as usize + 2)
                };
                msix = Some(MsixCap { cap_offset: cap_ptr, msg_ctrl });
            }
            CAP_ID_PM => {
                pm = Some(PmCap { cap_offset: cap_ptr });
            }
            _ => {}
        }
        cap_ptr = next;
    }

    (msix, pm)
}

// ── Scanner ───────────────────────────────────────────────────────────────────

/// Scan bus 0 of the ECAM window and populate the global `PCI_DEVICES` list.
///
/// # Safety
/// `ecam_base` must point to an identity-mapped 1 MiB ECAM bus-0 window.
unsafe fn scan(ecam_base: usize) {
    let mut devices = PCI_DEVICES.lock();

    for dev in 0u8..32 {
        // Check function 0 first. If vendor == 0xFFFF, no device is present.
        // SAFETY: ECAM window is identity-mapped.
        let vendor = unsafe { read16(ecam_base, 0, dev, 0, CFG_VENDOR_ID) };
        if vendor == 0xFFFF {
            continue;
        }

        // SAFETY: ECAM window is identity-mapped.
        let hdr_type = unsafe { read8(ecam_base, 0, dev, 0, CFG_HEADER_TYPE) };
        // Bit 7 of header_type indicates a multi-function device.
        let is_multi = hdr_type & 0x80 != 0;
        let max_fun: u8 = if is_multi { 8 } else { 1 };

        for fun in 0u8..max_fun {
            // SAFETY: ECAM window is identity-mapped.
            let vid = unsafe { read16(ecam_base, 0, dev, fun, CFG_VENDOR_ID) };
            if vid == 0xFFFF { continue; }

            // SAFETY: ECAM window is identity-mapped.
            let did      = unsafe { read16(ecam_base, 0, dev, fun, CFG_DEVICE_ID) };
            let prog_if  = unsafe { read8(ecam_base, 0, dev, fun, CFG_CLASS_PROG) };
            let subclass = unsafe { read8(ecam_base, 0, dev, fun, CFG_SUBCLASS) };
            let class    = unsafe { read8(ecam_base, 0, dev, fun, CFG_CLASS_CODE) };

            // Decode BARs for type-0 headers (endpoints). Skip bridges (type-1).
            let hdr = unsafe { read8(ecam_base, 0, dev, fun, CFG_HEADER_TYPE) } & 0x7F;
            let bars = if hdr == 0 {
                // SAFETY: ECAM window is identity-mapped.
                unsafe { decode_bars(ecam_base, 0, dev, fun) }
            } else {
                [Bar::None; 6]
            };

            // Walk capabilities.
            // SAFETY: ECAM window is identity-mapped.
            let (msix, pm) = unsafe { walk_caps(ecam_base, 0, dev, fun) };

            let bar0_addr = bars[0].base_addr();
            log::info!(
                "[pcie] {:02x}:{:02x}.{} vendor={:04x} device={:04x} \
                 class={:02x}:{:02x}:{:02x} bar0={:#x}",
                0u8, dev, fun, vid, did, class, subclass, prog_if, bar0_addr
            );

            devices.push(PciDevice {
                bdf: (0, dev, fun),
                vendor_id: vid,
                device_id: did,
                class,
                subclass,
                prog_if,
                bars,
                msix,
                pm,
            });
        }
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Initialise the ECAM scanner.
///
/// Must be called after paging is active (ECAM window mapped) and before
/// `blk_nvme::init_driver()`. Idempotent; safe to call more than once (later
/// calls are no-ops if the list is already populated).
pub fn init() {
    // Determine ECAM base for the current architecture.
    #[cfg(target_arch = "x86_64")]
    let ecam_base = ECAM_BASE_X86;
    #[cfg(target_arch = "riscv64")]
    let ecam_base = ECAM_BASE_RISCV;
    #[cfg(target_arch = "aarch64")]
    let ecam_base = ECAM_BASE_AARCH64;
    #[cfg(not(any(target_arch = "x86_64", target_arch = "riscv64", target_arch = "aarch64")))]
    let ecam_base = 0usize; // bare-physical arches: no PCIe

    if ecam_base == 0 {
        log::info!("[pcie] ECAM: no PCIe on this architecture");
        return;
    }

    // If already populated (e.g. called twice), skip re-scan.
    if !PCI_DEVICES.lock().is_empty() {
        log::info!("[pcie] ECAM: already scanned, skipping");
        return;
    }

    log::info!("[pcie] ECAM scan bus 0 @ {:#x} (1 MiB window)", ecam_base);

    // SAFETY: The ECAM bus-0 window (1 MiB at ecam_base) is identity-mapped in
    // `init_kernel_paging` (riscv/arm) or `init_kernel_paging_x86` (x86_64)
    // before this function is called. Volatile config-space reads/writes are
    // bounded to that window.
    unsafe { scan(ecam_base); }

    let count = PCI_DEVICES.lock().len();
    if count == 0 {
        log::warn!("[pcie] ECAM scan found no devices on bus 0 — check ECAM base and MMIO mapping");
    } else {
        log::info!("[pcie] ECAM scan complete: {} device(s) found", count);
    }
}

/// Return a cloned snapshot of the discovered device list.
pub fn devices() -> Vec<PciDevice> {
    PCI_DEVICES.lock().clone()
}

/// Find the first device matching (class, subclass, prog_if), or `None`.
///
/// Example: NVMe = `find_class(0x01, 0x08, 0x02)`.
pub fn find_class(class: u8, subclass: u8, prog_if: u8) -> Option<PciDevice> {
    PCI_DEVICES
        .lock()
        .iter()
        .find(|d| d.class == class && d.subclass == subclass && d.prog_if == prog_if)
        .cloned()
}
