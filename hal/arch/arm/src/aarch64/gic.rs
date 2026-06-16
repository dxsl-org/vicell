//! GIC-400 (GICv2) driver for QEMU virt machine.
//!
//! Distributor: 0x08000000  CPU interface: 0x08010000
//! Use  in QEMU to select GICv2.

const GICD_BASE: usize = 0x0800_0000;
const GICC_BASE: usize = 0x0801_0000;

fn gicd(offset: usize) -> *mut u32 { (GICD_BASE + offset) as *mut u32 }
fn gicc(offset: usize) -> *mut u32 { (GICC_BASE + offset) as *mut u32 }

fn wr(ptr: *mut u32, val: u32) { unsafe { core::ptr::write_volatile(ptr, val) } }
fn rd(ptr: *mut u32) -> u32    { unsafe { core::ptr::read_volatile(ptr) } }

const GICD_CTLR:   usize = 0x000;
const GICD_ISENABLER: usize = 0x100; // +4*n
const GICD_IPRIORITYR: usize = 0x400;
const GICD_ITARGETSR: usize = 0x800;
const GICD_ICFGR:  usize = 0xC00;
const GICC_CTLR:   usize = 0x000;
const GICC_PMR:    usize = 0x004;
const GICC_IAR:    usize = 0x00C;
const GICC_EOIR:   usize = 0x010;

/// Initialise GIC distributor and CPU interface.
pub fn init() {
    // Disable distributor, configure, then enable.
    wr(gicd(GICD_CTLR), 0);

    // Set all SPIs to edge-triggered, targeting CPU 0, medium priority.
    let lines = (((rd(gicd(0x004)) & 0x1F) + 1) * 32) as usize; // GICD_TYPER
    for i in 0..(lines / 4) {
        wr(gicd(GICD_IPRIORITYR + i * 4), 0xA0A0_A0A0);
        wr(gicd(GICD_ITARGETSR  + i * 4), 0x0101_0101); // CPU 0
    }
    for i in 0..(lines / 16) {
        wr(gicd(GICD_ICFGR + i * 4), 0); // level-triggered
    }

    // Enable distributor.
    wr(gicd(GICD_CTLR), 1);

    // Enable VirtIO MMIO IRQs: QEMU virt assigns SPI 16..47 (GIC IDs 48..79)
    // to the 32 VirtIO MMIO slots.  Without this, GICD_ISENABLER bit is 0 and
    // the GIC never delivers VirtIO interrupts even after claim/complete.
    // NIC is at slot 30 (SPI 46, GIC ID 78); Block at slot 31 (SPI 47, GIC ID 79).
    for i in 48u32..80 {
        enable_irq(i);
    }

    // CPU interface: allow all priorities, enable.
    wr(gicc(GICC_PMR), 0xFF);
    wr(gicc(GICC_CTLR), 1);
}

/// Enable a specific IRQ in the distributor.
pub fn enable_irq(irq: u32) {
    let reg = GICD_ISENABLER + (irq as usize / 32) * 4;
    wr(gicd(reg), 1 << (irq % 32));
}

/// Claim the highest-priority pending IRQ (acknowledge).
///
/// Returns the IRQ ID, or 0x3FF if no interrupt is pending.
pub fn claim() -> u32 {
    rd(gicc(GICC_IAR)) & 0x3FF
}

/// Signal end-of-interrupt for .
pub fn complete(irq: u32) {
    wr(gicc(GICC_EOIR), irq);
}
