//! Platform peripheral discovery via DTB.
//!
//! Call `platform::init(dtb_ptr)` at `kmain` before any driver init. All drivers
//! read MMIO addresses via `platform::with(|p| p.xxx)`. Falls back to QEMU virt
//! defaults when the DTB is absent or a compatible node is not found.
//!
//! Call ordering invariant: `init` → `uart::init` → `hal::ARCH.init` (which calls
//! `plic::init` internally). Any call to `with` before `init` panics.

use crate::sync::Spinlock;

// ── QEMU virt defaults (riscv64 fallback) ─────────────────────────────────────

const DEFAULT_UART_BASE:  usize = 0x1000_0000;
const DEFAULT_UART_IRQ:   u32   = 10;
const DEFAULT_PLIC_BASE:  usize = 0x0C00_0000;
/// 64 MB: PLIC claim/complete registers are at base + 0x20_0000 * context.
const DEFAULT_PLIC_SIZE:  usize = 0x400_0000;
const DEFAULT_CLINT_BASE: usize = 0x0200_0000;
/// Goldfish RTC default on QEMU RISC-V virt (google,goldfish-rtc in DTB).
const DEFAULT_RTC_BASE:   usize = 0x0010_1000;

// ── Public types ───────────────────────────────────────────────────────────────

/// A VirtIO MMIO device found in the DTB.
#[derive(Clone, Copy)]
pub struct VirtioEntry {
    pub base: usize,
    pub irq:  u32,
}

/// Platform peripheral layout populated once from the DTB at early boot.
#[derive(Clone)]
pub struct PlatformInfo {
    pub uart_base:   usize,
    pub uart_irq:    u32,
    pub plic_base:   usize,
    /// Mapped region size for identity-map range covering all PLIC registers.
    pub plic_size:   usize,
    pub clint_base:  usize,
    /// VirtIO MMIO slots from DTB (up to 8). `None` = slot unused.
    pub virtio_mmio: [Option<VirtioEntry>; 8],
    /// Goldfish RTC MMIO base (0 = not found in DTB, using default).
    pub rtc_base:    usize,
}

impl PlatformInfo {
    fn qemu_defaults() -> Self {
        Self {
            uart_base:   DEFAULT_UART_BASE,
            uart_irq:    DEFAULT_UART_IRQ,
            plic_base:   DEFAULT_PLIC_BASE,
            plic_size:   DEFAULT_PLIC_SIZE,
            clint_base:  DEFAULT_CLINT_BASE,
            virtio_mmio: [
                Some(VirtioEntry { base: 0x1000_1000, irq: 1 }),
                Some(VirtioEntry { base: 0x1000_2000, irq: 2 }),
                Some(VirtioEntry { base: 0x1000_3000, irq: 3 }),
                Some(VirtioEntry { base: 0x1000_4000, irq: 4 }),
                Some(VirtioEntry { base: 0x1000_5000, irq: 5 }),
                None, None, None,
            ],
            rtc_base:    DEFAULT_RTC_BASE,
        }
    }
}

// ── Storage ────────────────────────────────────────────────────────────────────

static PLATFORM: Spinlock<Option<PlatformInfo>> = Spinlock::new(None);

// ── Public API ─────────────────────────────────────────────────────────────────

/// Parse the DTB and store platform info. Must be called before `uart::init`,
/// `plic::set_plic_base`, and `init_kernel_paging`. Safe with `dtb_ptr == 0`.
#[cfg(target_arch = "riscv64")]
pub fn init(sbi_dtb: usize) {
    // Prefer the DTB pointer from a Limine DtbResponse (set by get_dtb_ptr
    // in boot/limine.rs before kmain). Falls back to the a1 register value.
    let dtb_ptr = match crate::boot::limine::get_dtb_ptr() {
        Some(p) => p,
        None    => sbi_dtb,
    };
    let info = from_dtb(dtb_ptr);
    log::info!("[platform] UART={:#x} irq={} PLIC={:#x}+{:#x} CLINT={:#x} RTC={:#x}",
        info.uart_base, info.uart_irq, info.plic_base, info.plic_size, info.clint_base, info.rtc_base);
    hal::common::rtc::init(info.rtc_base);
    *PLATFORM.lock() = Some(info);
}

// ── QEMU ARM virt defaults (aarch64) ─────────────────────────────────────────
// QEMU ARM virt: 32 VirtIO MMIO slots at 0x0a000000, 512 bytes each, SPI 16+i.
// Goldfish RTC at 0x0902_0000 on ARM virt; UART (PL011) at 0x0900_0000.
#[cfg(target_arch = "aarch64")]
pub fn init(_dtb_ptr: usize) {
    hal::rtc::init_default();
    *PLATFORM.lock() = Some(PlatformInfo {
        uart_base:   0x0900_0000,
        uart_irq:    1,
        plic_base:   0,
        plic_size:   0,
        clint_base:  0,
        virtio_mmio: [
            Some(VirtioEntry { base: 0x0a00_0000, irq: 16 }),
            Some(VirtioEntry { base: 0x0a00_0200, irq: 17 }),
            Some(VirtioEntry { base: 0x0a00_0400, irq: 18 }),
            Some(VirtioEntry { base: 0x0a00_0600, irq: 19 }),
            None, None, None, None,
        ],
        rtc_base:    0x0902_0000,
    });
}

#[cfg(not(any(target_arch = "riscv64", target_arch = "aarch64")))]
pub fn init(_dtb_ptr: usize) {}

/// Borrow the platform info. Panics if `init` was not called.
pub fn with<R>(f: impl FnOnce(&PlatformInfo) -> R) -> R {
    let guard = PLATFORM.lock();
    f(guard.as_ref().expect("[platform] platform::init not called before platform::with"))
}

// ── DTB parser (riscv64 only) ──────────────────────────────────────────────────

#[cfg(target_arch = "riscv64")]
fn from_dtb(dtb_ptr: usize) -> PlatformInfo {
    if dtb_ptr == 0 {
        log::warn!("[platform] dtb_ptr=0, using QEMU defaults");
        return PlatformInfo::qemu_defaults();
    }
    // SAFETY: dtb_ptr is the FDT physical address passed by OpenSBI (a1) or
    // retrieved from a Limine DtbResponse. fdt::Fdt::from_ptr validates FDT
    // magic before any further parsing.
    let fdt = match unsafe { fdt::Fdt::from_ptr(dtb_ptr as *const u8) } {
        Ok(f)  => f,
        Err(e) => {
            log::warn!("[platform] DTB parse error ({:?}), using QEMU defaults", e);
            return PlatformInfo::qemu_defaults();
        }
    };

    let uart_base = reg_base(&fdt, &["ns16550a", "ns16550"])
        .unwrap_or_else(|| { log::warn!("[platform] UART not in DTB"); DEFAULT_UART_BASE });
    let uart_irq  = irq_first(&fdt, &["ns16550a", "ns16550"])
        .unwrap_or(DEFAULT_UART_IRQ);

    let (plic_base, plic_size) = reg_base_size(&fdt, &["sifive,plic-1.0.0", "riscv,plic0"])
        .unwrap_or_else(|| { log::warn!("[platform] PLIC not in DTB"); (DEFAULT_PLIC_BASE, DEFAULT_PLIC_SIZE) });

    let clint_base = reg_base(&fdt, &["sifive,clint0", "riscv,clint0"])
        .unwrap_or_else(|| { log::warn!("[platform] CLINT not in DTB"); DEFAULT_CLINT_BASE });

    let virtio_mmio = collect_virtio(&fdt);

    let rtc_base = reg_base(&fdt, &["google,goldfish-rtc"])
        .unwrap_or_else(|| { log::warn!("[platform] Goldfish RTC not in DTB, using default"); DEFAULT_RTC_BASE });

    PlatformInfo { uart_base, uart_irq, plic_base, plic_size, clint_base, virtio_mmio, rtc_base }
}

#[cfg(target_arch = "riscv64")]
fn reg_base(fdt: &fdt::Fdt, compat: &[&str]) -> Option<usize> {
    reg_base_size(fdt, compat).map(|(b, _)| b)
}

#[cfg(target_arch = "riscv64")]
fn reg_base_size(fdt: &fdt::Fdt, compat: &[&str]) -> Option<(usize, usize)> {
    let node = fdt.find_compatible(compat)?;
    let r    = node.reg()?.next()?;
    Some((r.starting_address as usize, r.size.unwrap_or(0x1000)))
}

/// Read the first cell of the `interrupts` property as a big-endian u32.
#[cfg(target_arch = "riscv64")]
fn irq_first(fdt: &fdt::Fdt, compat: &[&str]) -> Option<u32> {
    let node = fdt.find_compatible(compat)?;
    let b    = node.property("interrupts")?.value;
    if b.len() >= 4 {
        Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    } else {
        None
    }
}

/// Collect all `virtio,mmio` nodes (up to 8) in DTB traversal order.
#[cfg(target_arch = "riscv64")]
fn collect_virtio(fdt: &fdt::Fdt) -> [Option<VirtioEntry>; 8] {
    let mut entries = [None, None, None, None, None, None, None, None];
    let mut n = 0;
    for node in fdt.all_nodes() {
        if n >= 8 { break; }
        let is_v = node.compatible()
            .map_or(false, |c| c.all().any(|s| s == "virtio,mmio"));
        if !is_v { continue; }
        let base = node.reg().and_then(|mut r| r.next())
            .map(|r| r.starting_address as usize);
        let irq = node.property("interrupts").and_then(|p| {
            let b = p.value;
            if b.len() >= 4 { Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]])) } else { None }
        });
        if let (Some(base), Some(irq)) = (base, irq) {
            entries[n] = Some(VirtioEntry { base, irq });
            n += 1;
        }
    }
    entries
}
