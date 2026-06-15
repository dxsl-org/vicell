//! GICv2 Virtual CPU Interface Control (GICH) MMIO register access for EL2 VMM.
//!
//! GICH base @ 0x0803_0000 (QEMU virt board, `gic-version=2 virtualization=on`).
//! All functions are `unsafe` — caller must be at EL2 with GICH MMIO accessible.
//!
//! # GICH_LR encoding for a Group1 Pending virtual IRQ
//! `[30]=Group1=1, [29:28]=Pending=0b01, [27:23]=Priority=0, [9:0]=VIntID`
//! → value = `0x5000_0000 | intid`
//!
//! # GICV Stage-2 passthrough
//! The caller maps guest GICC IPA (0x0801_0000) → host GICV HPA (0x0804_0000)
//! in the Stage-2 table so guest GICC reads/writes hit real GICV hardware without
//! causing a VM exit.  Phase 09 implements both: GICH LR injection + GICV passthrough.

const GICH_BASE: usize = 0x0803_0000;
const GICH_HCR:    usize = GICH_BASE;          // +0x000
const GICH_VTR:    usize = GICH_BASE + 0x004;  // +0x004
const GICH_ELRSR0: usize = GICH_BASE + 0x030;  // +0x030
const GICH_LR_BASE: usize = GICH_BASE + 0x100; // +0x100, LR[n] at +n*4

/// Maximum list registers to use; QEMU GICv2 typically has 4.
pub const MAX_LRS: usize = 4;

#[inline]
unsafe fn rd32(addr: usize) -> u32 {
    // SAFETY: caller guarantees EL2 + valid MMIO address.
    core::ptr::read_volatile(addr as *const u32)
}

#[inline]
unsafe fn wr32(addr: usize, val: u32) {
    // SAFETY: caller guarantees EL2 + valid MMIO address.
    core::ptr::write_volatile(addr as *mut u32, val);
}

/// Enable GICH (set GICH_HCR.EN = 1).
///
/// # Safety
/// Caller must be at EL2; GICH MMIO at 0x0803_0000 must be accessible.
pub unsafe fn enable() {
    let cur = rd32(GICH_HCR);
    wr32(GICH_HCR, cur | 1);
}

/// Return the number of list registers (GICH_VTR[4:0] + 1), capped at MAX_LRS.
///
/// # Safety
/// Caller must be at EL2; GICH MMIO must be accessible.
pub unsafe fn lr_count() -> usize {
    let vtr = rd32(GICH_VTR);
    (((vtr & 0x1F) + 1) as usize).min(MAX_LRS)
}

/// Load GICH_LR[`n`] with a Group1 Pending virtual IRQ for `intid`.
///
/// Precondition: `intid` ≤ 1019 (validated by syscall layer, m3).
///
/// # Safety
/// Caller must be at EL2; `n` < `lr_count()`.
pub unsafe fn load_lr(n: usize, intid: u32) {
    // Group1=1 (bit30), State=Pending (bit28), Priority=0, VIntID=intid[9:0].
    wr32(GICH_LR_BASE + n * 4, 0x5000_0000 | (intid & 0x3FF));
}

/// Read raw value of GICH_LR[`n`].
///
/// # Safety
/// Caller must be at EL2; `n` < `lr_count()`.
pub unsafe fn read_lr(n: usize) -> u32 {
    rd32(GICH_LR_BASE + n * 4)
}

/// Clear GICH_LR[`n`] (write 0 → Invalid state, slot becomes empty).
///
/// # Safety
/// Caller must be at EL2; `n` < `lr_count()`.
pub unsafe fn clear_lr(n: usize) {
    wr32(GICH_LR_BASE + n * 4, 0);
}

/// Read GICH_ELRSR0: bit N = 1 means LR N is empty (can accept a new virtual IRQ).
///
/// # Safety
/// Caller must be at EL2.
pub unsafe fn read_elrsr() -> u32 {
    rd32(GICH_ELRSR0)
}
