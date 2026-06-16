//! AArch64 vCPU register bank and world-switch for the ViCell EL2 VMM (Phase 03).
//!
//! # World-switch overview
//!
//! **Entry (`vcpu_enter_guest` asm):**
//! 1. Save host callee-saved (x19-x30) + SP_EL2 to `vcpu.h_*` fields.
//! 2. Store `vcpu` ptr in `TPIDR_EL2` (the guest-trap trampoline reads this).
//! 3. Set `HCR_EL2` to guest bits: `RW | VM | SWIO | AMO | IMO | FMO | TWI | TWE | TSC`.
//! 4. Restore guest GP (x1-x30), then x0 last (overwrites vcpu ptr).
//! 5. `eret` → guest EL1.
//!
//! **Exit (`vt_vcpu_trap` asm in this module, branched to from `el2.rs::vt_sync_el2_lower`):**
//! 1. Save guest x0-x30 + exit regs (ESR/ELR/FAR/HPFAR) to `vcpu.*`.
//! 2. Restore host `HCR_EL2 = RW | TGE` (clears VM — mandatory before any Cell access).
//! 3. Clear `TPIDR_EL2` (marks no active guest).
//! 4. Restore host callee-saved (x19-x30) + SP_EL2 from `vcpu.h_*`.
//! 5. `ret` → returns to `run_vcpu_impl` as if `vcpu_enter_guest` returned.
//!
//! # AArch64Vcpu layout (fixed `#[repr(C)]`, offsets verified by `offset_check` cfg)
//! ```text
//!   0 .. 248  gp[0..30]     guest x0-x30
//! 248 .. 280  exit_*        ESR/ELR/FAR/HPFAR saved on each trap
//! 280 .. 400  g_sysregs     15 × u64 guest EL1 sysreg bank
//! 400 .. 416  g_el2_ctrl    ELR_EL2 + SPSR_EL2 for guest entry
//! 416 .. 512  h_*           host callee-saved (x19-x30) + SP
//! 512 .. 520  h_sp          host SP_EL2 (separate field for alignment)
//! ```
//!
//! # FP handling
//! MVP: eager save/restore is NOT implemented for Phase 03 smoke tests.
//! The smoke guest uses no SIMD.  Phase 05 adds lazy CPTR_EL2.TFP management.

use core::arch::global_asm;
use hal_hypervisor::ViVmExit;

// ── AArch64Vcpu ──────────────────────────────────────────────────────────────

/// AArch64 vCPU register bank.
///
/// **Safety:** `#[repr(C)]` layout is part of the ABI between Rust and the
/// inline assembly trampolines.  Do not reorder or remove fields without
/// updating every byte-offset constant in the `global_asm!` block below.
#[derive(Debug, Default)]
#[repr(C)]
pub struct AArch64Vcpu {
    // ─── Guest GP registers x0-x30 (offsets 0..248) ─────────────────
    pub gp: [u64; 31],           // 31 × 8 = 248 bytes

    // ─── VM exit info (offsets 248..280) ────────────────────────────
    pub exit_esr:   u64,         // 248 — ESR_EL2 on trap
    pub exit_elr:   u64,         // 256 — ELR_EL2 = guest PC at trap
    pub exit_far:   u64,         // 264 — FAR_EL2
    pub exit_hpfar: u64,         // 272 — HPFAR_EL2

    // ─── Guest EL1 sysreg bank (offsets 280..400) ───────────────────
    pub g_sctlr_el1:  u64,      // 280
    pub g_ttbr0_el1:  u64,      // 288
    pub g_ttbr1_el1:  u64,      // 296
    pub g_tcr_el1:    u64,      // 304
    pub g_mair_el1:   u64,      // 312
    pub g_vbar_el1:   u64,      // 320
    pub g_esr_el1:    u64,      // 328
    pub g_far_el1:    u64,      // 336
    pub g_tpidr_el0:  u64,      // 344
    pub g_tpidr_el1:  u64,      // 352
    pub g_cntv_ctl:   u64,      // 360
    pub g_cntv_cval:  u64,      // 368
    pub g_spsr_el1:   u64,      // 376
    pub g_elr_el1:    u64,      // 384
    pub g_sp_el1:     u64,      // 392

    // ─── Guest EL2 entry control (offsets 400..416) ─────────────────
    pub g_elr_el2:  u64,        // 400 — guest entry/resume PC
    pub g_spsr_el2: u64,        // 408 — SPSR_EL2 on entry (0x3C5 = EL1h+DAIF)

    // ─── Host save area (offsets 416..520) ──────────────────────────
    // Callee-saved registers (x19-x30) and SP_EL2 saved by vcpu_enter_guest;
    // restored by vt_vcpu_trap so that `ret` returns to run_vcpu_impl.
    pub h_x19: u64, pub h_x20: u64, // 416, 424
    pub h_x21: u64, pub h_x22: u64, // 432, 440
    pub h_x23: u64, pub h_x24: u64, // 448, 456
    pub h_x25: u64, pub h_x26: u64, // 464, 472
    pub h_x27: u64, pub h_x28: u64, // 480, 488
    pub h_x29: u64, pub h_x30: u64, // 496, 504 — h_x30 = host return address
    pub h_sp:  u64,                  // 512 — host SP_EL2
}

// Verify critical struct offsets match the asm constants at compile time.
// SAFETY: these are purely compile-time size/offset checks.
const _: () = {
    assert!(core::mem::offset_of!(AArch64Vcpu, gp)         == 0);
    assert!(core::mem::offset_of!(AArch64Vcpu, exit_esr)   == 248);
    assert!(core::mem::offset_of!(AArch64Vcpu, exit_elr)   == 256);
    assert!(core::mem::offset_of!(AArch64Vcpu, exit_far)   == 264);
    assert!(core::mem::offset_of!(AArch64Vcpu, exit_hpfar) == 272);
    assert!(core::mem::offset_of!(AArch64Vcpu, g_elr_el2)  == 400);
    assert!(core::mem::offset_of!(AArch64Vcpu, g_spsr_el2) == 408);
    assert!(core::mem::offset_of!(AArch64Vcpu, h_x19)      == 416);
    assert!(core::mem::offset_of!(AArch64Vcpu, h_x29)      == 496);
    assert!(core::mem::offset_of!(AArch64Vcpu, h_x30)      == 504);
    assert!(core::mem::offset_of!(AArch64Vcpu, h_sp)       == 512);
};

// SAFETY: AArch64Vcpu is only accessed from single-CPU kernel context in Phase 03.
// SMP safety is deferred to Phase 05 (one Vcpu per CPU with per-CPU TPIDR_EL2).
unsafe impl Send for AArch64Vcpu {}

impl AArch64Vcpu {
    /// Create a new vCPU with the guest starting at `entry` in AArch64 EL1h mode.
    ///
    /// SPSR_EL2 = 0x3C5: EL1h (SP_EL1), all DAIF bits masked, AArch64 state.
    /// All GP registers default to 0; EL1 sysregs default to safe reset values.
    pub fn new(entry: u64) -> Self {
        let mut v = Self::default();
        v.g_elr_el2  = entry;
        v.g_spsr_el2 = 0x3C5; // EL1h, DAIF=1111
        v
    }

    /// Decode the VM exit recorded on the last trap.
    pub fn decode_exit(&self) -> ViVmExit {
        super::trap_el2::decode_vmexit(
            self.exit_esr,
            self.exit_elr,
            self.exit_far,
            self.exit_hpfar,
            &self.gp,
        )
    }
}

impl Drop for AArch64Vcpu {
    // Stage2Table is owned by the caller — no EL2-sensitive resource to release here.
    fn drop(&mut self) {}
}

// ── Host EL1 sysreg snapshot (saved on stack during guest run) ───────────────

/// Host EL1 sysreg state saved/restored around each `run_vcpu_impl` call.
#[derive(Default)]
#[repr(C)]
struct HostEl1Bank {
    sctlr_el1: u64, ttbr0_el1: u64, ttbr1_el1: u64, tcr_el1: u64,
    mair_el1:  u64, vbar_el1:  u64, tpidr_el0:  u64, tpidr_el1: u64,
    cntv_ctl:  u64, cntv_cval: u64, spsr_el1:   u64, elr_el1:  u64,
    sp_el1:    u64,
}

// ── run_vcpu_impl ─────────────────────────────────────────────────────────────

/// Run the vCPU until a VM exit occurs, returning the decoded exit reason.
///
/// # Safety
/// - Must be called from EL2 (`el2::is_el2()` == true).
/// - Stage-2 translation must be enabled for `vcpu`'s VMID before this call.
/// - `vcpu` must remain valid (pinned in memory) for the duration of the call;
///   the `vt_vcpu_trap` assembly trampoline dereferences its pointer mid-exception.
/// - Only one vcpu may be running per CPU at a time (TPIDR_EL2 is per-CPU only
///   in a single-CPU QEMU TCG environment; SMP requires per-CPU vcpu tracking).
pub unsafe fn run_vcpu_impl(vcpu: &mut AArch64Vcpu) -> ViVmExit {
    // ── 1. Snapshot host EL1 sysregs ────────────────────────────────────────
    let mut h = HostEl1Bank::default();
    // SAFETY: reading EL1/EL0 sysregs from EL2 is unconditionally permitted.
    unsafe {
        core::arch::asm!(
            "mrs {0}, sctlr_el1",  "mrs {1}, ttbr0_el1",
            "mrs {2}, ttbr1_el1",  "mrs {3}, tcr_el1",
            "mrs {4}, mair_el1",   "mrs {5}, vbar_el1",
            "mrs {6}, tpidr_el0",  "mrs {7}, tpidr_el1",
            out(reg) h.sctlr_el1, out(reg) h.ttbr0_el1,
            out(reg) h.ttbr1_el1, out(reg) h.tcr_el1,
            out(reg) h.mair_el1,  out(reg) h.vbar_el1,
            out(reg) h.tpidr_el0, out(reg) h.tpidr_el1,
            options(nomem, nostack),
        );
        core::arch::asm!(
            "mrs {0}, cntv_ctl_el0",  "mrs {1}, cntv_cval_el0",
            "mrs {2}, spsr_el1",      "mrs {3}, elr_el1",
            "mrs {4}, sp_el1",
            out(reg) h.cntv_ctl, out(reg) h.cntv_cval,
            out(reg) h.spsr_el1, out(reg) h.elr_el1,
            out(reg) h.sp_el1,
            options(nomem, nostack),
        );
    }

    // ── 2a. Configure virtual-timer access for guest ─────────────────────────
    // CNTVOFF_EL2 = 0: virtual timer offset from physical counter = 0.
    // CNTHCTL_EL2 = 0b11: EL1PCTEN|EL1PCEN — EL1/EL0 may read physical counter.
    // Linux uses the virtual timer (CNTV / PPI 27); these two writes enable it.
    // SAFETY: EL2-private sysregs; safe to write unconditionally before guest entry.
    unsafe {
        core::arch::asm!(
            "msr cntvoff_el2, xzr",
            "mov {0}, #3",
            "msr cnthctl_el2, {0}",
            out(reg) _,
            options(nomem, nostack),
        );
    }

    // ── 2. Restore guest EL1 sysregs + EL2 entry control ────────────────────
    // SAFETY: writing EL1/EL0 sysregs from EL2 is permitted; Cells are EL0
    // and protected by TGE routing — these writes only affect the guest bank.
    unsafe {
        core::arch::asm!(
            "msr sctlr_el1, {0}",  "msr ttbr0_el1, {1}",
            "msr ttbr1_el1, {2}",  "msr tcr_el1,   {3}",
            "msr mair_el1,  {4}",  "msr vbar_el1,  {5}",
            "msr tpidr_el0, {6}",  "msr tpidr_el1, {7}",
            in(reg) vcpu.g_sctlr_el1, in(reg) vcpu.g_ttbr0_el1,
            in(reg) vcpu.g_ttbr1_el1, in(reg) vcpu.g_tcr_el1,
            in(reg) vcpu.g_mair_el1,  in(reg) vcpu.g_vbar_el1,
            in(reg) vcpu.g_tpidr_el0, in(reg) vcpu.g_tpidr_el1,
            options(nomem, nostack),
        );
        core::arch::asm!(
            "msr cntv_ctl_el0,  {0}",  "msr cntv_cval_el0, {1}",
            "msr spsr_el1,      {2}",  "msr elr_el1,       {3}",
            "msr sp_el1,        {4}",
            in(reg) vcpu.g_cntv_ctl,  in(reg) vcpu.g_cntv_cval,
            in(reg) vcpu.g_spsr_el1,  in(reg) vcpu.g_elr_el1,
            in(reg) vcpu.g_sp_el1,
            options(nomem, nostack),
        );
        // Program ELR_EL2 (guest PC) + SPSR_EL2 for `eret` in vcpu_enter_guest.
        core::arch::asm!(
            "msr elr_el2,  {0}",
            "msr spsr_el2, {1}",
            in(reg) vcpu.g_elr_el2,
            in(reg) vcpu.g_spsr_el2,
            options(nomem, nostack),
        );
    }

    // ── 3. Enter guest + wait for trap ───────────────────────────────────────
    // `vcpu_enter_guest` saves host callee-saved regs + SP to vcpu.h_*, stores
    // TPIDR_EL2 = vcpu, sets HCR_EL2 guest bits, loads guest GP, and erets.
    // `vt_vcpu_trap` (in global_asm! below) fires on guest trap: saves guest GP
    // + exit info to vcpu, restores host callee-saved + SP, clears TPIDR_EL2,
    // and `ret`s — returning here as if vcpu_enter_guest() returned normally.
    // SAFETY: Stage-2 enabled; TPIDR_EL2 used as single-CPU coroutine pointer.
    unsafe { vcpu_enter_guest(vcpu as *mut AArch64Vcpu); }

    // ── 4. Restore host EL1 sysregs ─────────────────────────────────────────
    // SAFETY: restoring our own host state; EL1 sysregs safe to write from EL2.
    unsafe {
        core::arch::asm!(
            "msr sctlr_el1, {0}",  "msr ttbr0_el1, {1}",
            "msr ttbr1_el1, {2}",  "msr tcr_el1,   {3}",
            "msr mair_el1,  {4}",  "msr vbar_el1,  {5}",
            "msr tpidr_el0, {6}",  "msr tpidr_el1, {7}",
            in(reg) h.sctlr_el1, in(reg) h.ttbr0_el1,
            in(reg) h.ttbr1_el1, in(reg) h.tcr_el1,
            in(reg) h.mair_el1,  in(reg) h.vbar_el1,
            in(reg) h.tpidr_el0, in(reg) h.tpidr_el1,
            options(nomem, nostack),
        );
        core::arch::asm!(
            "msr cntv_ctl_el0,  {0}",  "msr cntv_cval_el0, {1}",
            "msr spsr_el1,      {2}",  "msr elr_el1,       {3}",
            "msr sp_el1,        {4}",
            in(reg) h.cntv_ctl,  in(reg) h.cntv_cval,
            in(reg) h.spsr_el1,  in(reg) h.elr_el1,
            in(reg) h.sp_el1,
            options(nomem, nostack),
        );
    }

    // ── 5. Decode VM exit ────────────────────────────────────────────────────
    let exit = vcpu.decode_exit();

    // ── 6. Advance guest PC for exits that consumed the trapping instruction ─
    // ELR_EL2 (saved as exit_elr) points AT the trapping instruction.
    // For HVC and WFI the guest must not re-execute on re-entry; advance by 4.
    // For MMIO (data abort) the hypervisor cell handles register injection and
    // sets g_elr_el2 explicitly before re-entering.
    match exit {
        ViVmExit::Hvc { .. } | ViVmExit::Wfi => {
            vcpu.g_elr_el2 = vcpu.exit_elr.wrapping_add(4);
        }
        _ => {
            // Unknown / MMIO: caller decides whether and by how much to advance.
            vcpu.g_elr_el2 = vcpu.exit_elr;
        }
    }

    exit
}

// ── vcpu_enter_guest extern declaration ──────────────────────────────────────

extern "C" {
    /// AArch64 assembly world-switch entry point.
    ///
    /// Saves host callee-saved registers + SP to `vcpu.h_*`, stores the vcpu ptr
    /// in `TPIDR_EL2`, programs `HCR_EL2` for guest execution, restores guest GP
    /// registers (x1-x30, then x0 last), and executes `eret` into the guest.
    ///
    /// The function "returns" when `vt_vcpu_trap` fires during a guest exception:
    /// it restores host state and `ret`s using the `h_x30` (return address to the
    /// instruction after this `bl` in `run_vcpu_impl`).
    ///
    /// # Safety
    /// - Must be called from EL2 with ELR_EL2/SPSR_EL2 already set.
    /// - `vcpu` must be a valid, exclusively-owned `AArch64Vcpu`.
    fn vcpu_enter_guest(vcpu: *mut AArch64Vcpu);
}

// ── Assembly: vcpu_enter_guest + vt_vcpu_trap ─────────────────────────────────
//
// AArch64Vcpu field byte offsets (MUST match the struct definition above):
//   gp[0..30]  → offsets  0 .. 240   (gp[n] = n*8)
//   exit_esr   → offset  248
//   exit_elr   → offset  256
//   exit_far   → offset  264
//   exit_hpfar → offset  272
//   (sysregs 280..399 — not accessed from asm)
//   g_elr_el2  → offset  400   (not used in asm; set by run_vcpu_impl via msr)
//   g_spsr_el2 → offset  408   (not used in asm; set by run_vcpu_impl via msr)
//   h_x19      → offset  416
//   h_x20      → offset  424
//   h_x21      → offset  432
//   h_x22      → offset  440
//   h_x23      → offset  448
//   h_x24      → offset  456
//   h_x25      → offset  464
//   h_x26      → offset  472
//   h_x27      → offset  480
//   h_x28      → offset  488
//   h_x29      → offset  496
//   h_x30      → offset  504   ← host LR / return address
//   h_sp       → offset  512

global_asm!(r#"
    .section .text
    .global vcpu_enter_guest
    .balign 4
vcpu_enter_guest:
    // x0 = *mut AArch64Vcpu
    // Save host callee-saved registers (AArch64 ABI: x19-x30) + SP_EL2.
    stp  x19, x20, [x0, #416]
    stp  x21, x22, [x0, #432]
    stp  x23, x24, [x0, #448]
    stp  x25, x26, [x0, #464]
    stp  x27, x28, [x0, #480]
    stp  x29, x30, [x0, #496]  // h_x29=FP, h_x30=return-addr-to-run_vcpu_impl
    mov  x9,  sp
    str  x9,       [x0, #512]  // h_sp = current SP_EL2

    // Store vcpu ptr in TPIDR_EL2 so vt_vcpu_trap can save guest state.
    // SAFETY: TPIDR_EL2 is EL2-private; the guest cannot read or write it.
    msr  tpidr_el2, x0

    // Set HCR_EL2 guest bits: RW|VM|SWIO|AMO|IMO|FMO|TWI|TWE|TSC.
    // RW(31)=AArch64 EL1, VM(0)=enable Stage-2, SWIO(1)=SW IRQ override,
    // AMO(3)/IMO(4)/FMO(5)=route physical async exceptions, TWI(12)/TWE(13)=trap
    // WFI/WFE to EL2 (lets us emulate them), TSC(19)=trap SMC to EL2.
    // SAFETY: HCR_EL2 is EL2-private.
    mov  x9,  #(1 << 31)       // RW
    orr  x9,  x9,  #(1 << 0)   // VM
    orr  x9,  x9,  #(1 << 1)   // SWIO
    orr  x9,  x9,  #(1 << 3)   // AMO
    orr  x9,  x9,  #(1 << 4)   // IMO
    orr  x9,  x9,  #(1 << 5)   // FMO
    orr  x9,  x9,  #(1 << 12)  // TWI
    orr  x9,  x9,  #(1 << 13)  // TWE
    orr  x9,  x9,  #(1 << 19)  // TSC
    msr  hcr_el2, x9
    isb

    // Restore guest GP registers.  Load x1-x30 first; x0 last (it's the vcpu ptr).
    ldp  x1,  x2,  [x0, #8]
    ldp  x3,  x4,  [x0, #24]
    ldp  x5,  x6,  [x0, #40]
    ldp  x7,  x8,  [x0, #56]
    ldp  x9,  x10, [x0, #72]
    ldp  x11, x12, [x0, #88]
    ldp  x13, x14, [x0, #104]
    ldp  x15, x16, [x0, #120]
    ldp  x17, x18, [x0, #136]
    ldp  x19, x20, [x0, #152]
    ldp  x21, x22, [x0, #168]
    ldp  x23, x24, [x0, #184]
    ldp  x25, x26, [x0, #200]
    ldp  x27, x28, [x0, #216]
    ldp  x29, x30, [x0, #232]
    ldr  x0,       [x0, #0]    // guest x0 — overwrites vcpu ptr in x0

    eret


    // ── vt_vcpu_trap ─────────────────────────────────────────────────────────
    //
    // Called from vt_sync_el2_lower (el2.rs) when TPIDR_EL2 != 0.
    //
    // On entry (set up by vt_sync_el2_lower):
    //   x0        = vcpu ptr (from TPIDR_EL2)
    //   [sp + 0]  = guest x0 (saved before we clobbered x0 to read TPIDR_EL2)
    //   [sp + 8]  = guest x1 (saved before we clobbered x1)
    //   sp        = host_SP - 16 (two scratch slots from vt_sync_el2_lower)
    //   x2-x30    = guest register values (untouched since guest ran)
    //
    // Exit: restores host state and `ret`s to run_vcpu_impl.

    .section .text
    .global vt_vcpu_trap
    .balign 4
vt_vcpu_trap:
    // Recover guest x0 and x1 from the scratch area, save them to vcpu.gp[0..1].
    ldr  x1, [sp, #8]          // guest x1 (we'll use x1 as a scratch reg here)
    str  x1, [x0, #8]          // vcpu.gp[1] = guest x1
    ldr  x1, [sp]              // guest x0 (reuse x1 as scratch)
    str  x1, [x0, #0]          // vcpu.gp[0] = guest x0
    add  sp, sp, #16            // undo the 16-byte scratch alloc from vt_sync_el2_lower

    // Save guest x2-x30 to vcpu.gp[2..30].
    stp  x2,  x3,  [x0, #16]
    stp  x4,  x5,  [x0, #32]
    stp  x6,  x7,  [x0, #48]
    stp  x8,  x9,  [x0, #64]
    stp  x10, x11, [x0, #80]
    stp  x12, x13, [x0, #96]
    stp  x14, x15, [x0, #112]
    stp  x16, x17, [x0, #128]
    stp  x18, x19, [x0, #144]
    stp  x20, x21, [x0, #160]
    stp  x22, x23, [x0, #176]
    stp  x24, x25, [x0, #192]
    stp  x26, x27, [x0, #208]
    stp  x28, x29, [x0, #224]
    str  x30,       [x0, #240]

    // Save VM exit info (ESR/ELR/FAR/HPFAR from EL2 sysregs).
    mrs  x1, esr_el2
    mrs  x2, elr_el2
    mrs  x3, far_el2
    mrs  x4, hpfar_el2
    stp  x1, x2, [x0, #248]    // exit_esr, exit_elr
    stp  x3, x4, [x0, #264]    // exit_far, exit_hpfar

    // Restore host HCR_EL2 = RW|TGE (no VM bit).
    // CRITICAL: must clear VM before Cell EL0 accesses run through Stage-2.
    // SAFETY: HCR_EL2 is EL2-private.
    mov  x1, #(1 << 31)
    orr  x1, x1, #(1 << 27)
    msr  hcr_el2, x1
    isb

    // Clear TPIDR_EL2 (no vCPU running now).
    msr  tpidr_el2, xzr

    // Restore host callee-saved registers + SP from vcpu.h_*.
    ldp  x19, x20, [x0, #416]
    ldp  x21, x22, [x0, #432]
    ldp  x23, x24, [x0, #448]
    ldp  x25, x26, [x0, #464]
    ldp  x27, x28, [x0, #480]
    ldp  x29, x30, [x0, #496]  // h_x29 = host FP, h_x30 = host return address
    ldr  x9,       [x0, #512]  // h_sp
    mov  sp, x9

    ret     // returns to run_vcpu_impl via restored h_x30
"#);
