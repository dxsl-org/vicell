//! ARM64 Stage-2 EL2 system-register helpers.
//!
//! All functions require the kernel to be running at EL2 (`el2::is_el2()` == true)
//! and must NOT be called before the Stage-2 page table is fully built and visible
//! to the CPU (all writes flushed to memory before the `isb`).
//!
//! # Register layout
//!
//! ## VTCR_EL2 (40-bit IPA, 4 KB granule, SL0=1 start at Level 1)
//! | Field | Bits | Value |
//! |-------|------|-------|
//! | T0SZ  | [5:0] | 24 (IPA = 2^40 = 1 TiB) |
//! | SL0   | [7:6] | 0b01 (start at L1) |
//! | IRGN0 | [9:8] | 0b01 (Normal WB-WA inner) |
//! | ORGN0 | [11:10] | 0b01 (Normal WB-WA outer) |
//! | SH0   | [13:12] | 0b11 (Inner-shareable) |
//! | PS    | [18:16] | 0b010 (40-bit PA size) |
//! | RES1  | [31] | 1 |
//!
//! Computed: `0x80023558`
//!
//! ## VTTBR_EL2
//! bits[63:48] = VMID (≥ 1; host is VMID 0)
//! bits[47:1]  = BADDR (root PA[47:13] for 8 KB-aligned root)
//!
//! ## HCR_EL2.VM (bit 0)
//! Setting this bit enables Stage-2 translation for EL0/EL1.
//! Must be cleared before tearing down the Stage-2 table.

/// VTCR_EL2 value for 40-bit IPA, 4 KB granule, Level-1 start (SL0=1),
/// Normal-WB-WA Inner+Outer-shareable, bit[31]=RES1.
///
/// T0SZ=24 | SL0=1 | IRGN0=01 | ORGN0=01 | SH0=11 | PS=010 | RES1
const VTCR_VALUE: u64 = 0x8002_3558;

/// Enable Stage-2 translation for the given `vmid` and root page-table at `root_pa`.
///
/// Programs VTCR_EL2 and VTTBR_EL2, sets HCR_EL2.VM=1, then issues a full
/// Stage-2 TLB invalidation to discard any stale cached translations.
///
/// # Safety
/// - Must only be called from EL2 (after `el2::el2_mark_active()`).
/// - `root_pa` must be 8 KB-aligned (concatenated 2 × L1 tables, VTCR.SL0=1).
/// - The Stage-2 page table must be fully populated and all writes flushed to
///   RAM before this function is called.
/// - `vmid` must be ≥ 1 (VMID 0 is reserved for the EL2 host).
/// - This function must NOT be called while a vCPU is running (HCR_EL2.VM races).
#[cfg(target_arch = "aarch64")]
pub unsafe fn enable_stage2(vmid: u16, root_pa: u64) {
    debug_assert_eq!(root_pa % (2 * 4096), 0, "S2 root must be 8 KB-aligned");
    debug_assert!(vmid >= 1, "VMID 0 is reserved for the EL2 host");

    let vttbr = ((vmid as u64) << 48) | root_pa;

    // SAFETY: caller guarantees EL2 context and well-formed root PA.
    unsafe {
        core::arch::asm!(
            // 1. Program VTCR_EL2 (S2 translation parameters).
            "msr vtcr_el2,  {vtcr}",
            // 2. Program VTTBR_EL2 (VMID + S2 root PA).
            "msr vttbr_el2, {vttbr}",
            "isb",
            // 3. Set HCR_EL2.VM = bit[0] to enable S2 translation.
            "mrs {tmp}, hcr_el2",
            "orr {tmp}, {tmp}, #1",
            "msr hcr_el2, {tmp}",
            "isb",
            // 4. Invalidate all Stage-1 + Stage-2 TLB entries for this VMID.
            "tlbi vmalls12e1is",
            "dsb ish",
            "isb",
            vtcr  = in(reg) VTCR_VALUE,
            vttbr = in(reg) vttbr,
            tmp   = out(reg) _,
            options(nomem, nostack),
        );
    }
}

/// Disable Stage-2 translation (clear HCR_EL2.VM=0) and flush the S2 TLB.
///
/// Call during VM teardown before freeing the Stage-2 page table frames.
///
/// # Safety
/// Must be called from EL2 with no vCPU currently running.
#[cfg(target_arch = "aarch64")]
pub unsafe fn disable_stage2() {
    // SAFETY: EL2 context; clearing HCR_EL2.VM disables guest translation.
    unsafe {
        core::arch::asm!(
            "mrs {tmp}, hcr_el2",
            "bic {tmp}, {tmp}, #1",   // clear VM bit
            "msr hcr_el2, {tmp}",
            "isb",
            "tlbi vmalls12e1is",
            "dsb ish",
            "isb",
            tmp = out(reg) _,
            options(nomem, nostack),
        );
    }
}

/// Invalidate all Stage-1 + Stage-2 TLB entries for all VMIDs (broadcast).
///
/// Use after bulk Stage-2 table changes or during VM teardown.
///
/// # Safety
/// Must be called from EL2.
#[cfg(target_arch = "aarch64")]
pub unsafe fn s2_tlb_flush_all() {
    // SAFETY: EL2 broadcast TLB invalidation; no memory mutation.
    unsafe {
        core::arch::asm!(
            "tlbi vmalls12e1is",
            "dsb ish",
            "isb",
            options(nomem, nostack),
        );
    }
}

/// Invalidate the Stage-2 TLB entry for a single guest IPA.
///
/// Encodes `ipa >> 12` into the `TLBI IPAS2E1IS` operand as required by the
/// ARM architecture (D8-7204: Xt = IPA[47:12]).  Also issues `TLBI VMALLE1IS`
/// to flush any Stage-1 entries cached during the S2 walk.
///
/// # Safety
/// Must be called from EL2 after updating the Stage-2 descriptor for `ipa`.
#[cfg(target_arch = "aarch64")]
pub unsafe fn s2_tlb_flush_ipa(ipa: u64) {
    let encoded = ipa >> 12; // TLBI IPAS2E1IS encodes IPA[47:12] in Xt
    // SAFETY: EL2 TLB maintenance; encoded IPA operand is correct per ARM DDI.
    unsafe {
        core::arch::asm!(
            "tlbi ipas2e1is, {x}",  // invalidate Stage-2 entry for this IPA
            "dsb ish",
            "tlbi vmalle1is",       // flush Stage-1 entries cached during S2 walk
            "dsb ish",
            "isb",
            x = in(reg) encoded,
            options(nomem, nostack),
        );
    }
}

// ── Non-AArch64 stubs (keeps workspace compiling on RISC-V / x86_64) ────────

#[cfg(not(target_arch = "aarch64"))]
pub unsafe fn enable_stage2(_vmid: u16, _root_pa: u64) {}

#[cfg(not(target_arch = "aarch64"))]
pub unsafe fn disable_stage2() {}

#[cfg(not(target_arch = "aarch64"))]
pub unsafe fn s2_tlb_flush_all() {}

#[cfg(not(target_arch = "aarch64"))]
pub unsafe fn s2_tlb_flush_ipa(_ipa: u64) {}
