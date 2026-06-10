//! PLIC (Platform-Level Interrupt Controller) Driver for RISC-V.
//! Reference: https://github.com/riscv/riscv-plic-spec/blob/master/riscv-plic.adoc

use core::sync::atomic::{AtomicUsize, Ordering};

pub const PLIC_BASE: usize = 0x0c00_0000;
pub const PLIC_PRIORITY_BASE: usize = 0x0;
pub const PLIC_PENDING_BASE: usize = 0x1000;
pub const PLIC_ENABLE_BASE: usize = 0x2000;
pub const PLIC_THRESHOLD_AND_CLAIM_BASE: usize = 0x20_0000;

// Context 0 is usually Hart 0 M-mode (often skipped in Linux/S-mode kernels if SBI handles M-mode)
// Context 1 is Hart 0 S-mode.
// For QEMU virt:
// Hart 0 M-mode: Context 0
// Hart 0 S-mode: Context 1
// Hart 1 M-mode: Context 2
// Hart 1 S-mode: Context 3
// ...
// We assume Single Core (Hart 0) S-mode for now -> Context 1.

/// Runtime PLIC base address. Updated before `init()` via `set_plic_base()`.
/// Defaults to QEMU virt layout (0x0C00_0000).
static PLIC_RUNTIME_BASE: AtomicUsize = AtomicUsize::new(PLIC_BASE);

/// Override the PLIC base address before `init()` is called (called from kernel
/// after DTB parsing populates `platform::PlatformInfo`).
pub fn set_plic_base(base: usize) {
    PLIC_RUNTIME_BASE.store(base, Ordering::Relaxed);
}

pub struct Plic;

impl Plic {
    pub const fn new(_base: usize) -> Self {
        Self
    }

    fn base() -> usize {
        PLIC_RUNTIME_BASE.load(Ordering::Relaxed)
    }

    /// Set priority for a specific IRQ.
    /// Priority: 0 (disabled) to 7 (highest).
    pub fn set_priority(&self, irq: u32, priority: u32) {
        let addr = Self::base() + PLIC_PRIORITY_BASE + (irq as usize) * 4;
        unsafe { (addr as *mut u32).write_volatile(priority); }
    }

    /// Enable interrupt for a specific Context.
    pub fn enable(&self, context: usize, irq: u32) {
        let addr = Self::base() + PLIC_ENABLE_BASE + (context * 0x80) + ((irq as usize / 32) * 4);
        let mask = 1 << (irq % 32);
        unsafe {
            let ptr = addr as *mut u32;
            ptr.write_volatile(ptr.read_volatile() | mask);
        }
    }

    /// Set priority threshold for a specific Context.
    /// Interrupts <= threshold are masked.
    pub fn set_threshold(&self, context: usize, threshold: u32) {
        let addr = Self::base() + PLIC_THRESHOLD_AND_CLAIM_BASE + (context * 0x1000);
        unsafe { (addr as *mut u32).write_volatile(threshold); }
    }

    /// Claim an interrupt for a specific Context.
    /// Returns the IRQ number, or 0 if none.
    pub fn claim(&self, context: usize) -> u32 {
        let addr = Self::base() + PLIC_THRESHOLD_AND_CLAIM_BASE + (context * 0x1000) + 4;
        unsafe { (addr as *mut u32).read_volatile() }
    }

    /// Complete an interrupt for a specific Context.
    pub fn complete(&self, context: usize, irq: u32) {
        let addr = Self::base() + PLIC_THRESHOLD_AND_CLAIM_BASE + (context * 0x1000) + 4;
        unsafe { (addr as *mut u32).write_volatile(irq); }
    }
}

// Global PLIC instance (zero-size; all state in PLIC_RUNTIME_BASE).
pub static PLIC: Plic = Plic::new(PLIC_BASE);

/// Initialize PLIC for Hart 0 S-Mode (Context 1).
/// Uses the base address set by `set_plic_base()` — call that first from kernel.
pub fn init() {
    PLIC.set_threshold(1, 0);
    // Enable VirtIO IRQs 1-8 (QEMU virt layout; same on JH7110).
    for irq in 1..=8 {
        PLIC.set_priority(irq, 1);
        PLIC.enable(1, irq);
    }
    // Enable UART0 (IRQ 10 on QEMU virt and JH7110).
    PLIC.set_priority(10, 1);
    PLIC.enable(1, 10);
}
