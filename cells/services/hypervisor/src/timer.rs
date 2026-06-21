//! Virtual timer + VI-bit interrupt injection policy.
//!
//! The ARM64 virtual timer (CNTV / PPI 27) is what Linux uses for scheduling.
//! The kernel sets CNTVOFF_EL2=0 at each vcpu entry (run_vcpu_impl, P05 addition),
//! so the guest counter matches the host physical counter.
//!
//! When the guest issues WFI (trapped by HCR_EL2.TWI), we check if the virtual
//! timer deadline has passed. If so, we inject PPI 27 via sys_inject_irq, which
//! sets HCR_EL2.VI (one pending virtual IRQ). On re-entry the guest sees the IRQ
//! and its timer handler fires.
//!
//! This is the "2 exits per timer IRQ" model: WFI → check → inject VI → re-enter →
//! guest IRQ handler runs → clears CNTV_CTL.ISTATUS → next WFI fires normally.

use api::syscall::ViSyscall;

/// GICv2 PPI 27 = virtual timer interrupt.
pub const VIRT_TIMER_PPI: u32 = 27;

/// Check if the guest virtual timer has fired (CNTV_CTL.ENABLE && CNTV_CVAL ≤ now).
///
/// `cntv_ctl` and `cntv_cval` are the guest shadow values read from vcpu registers
/// via `sys_vcpu_regs`. `cntpct` is the host physical counter value from `sys_get_time`.
pub fn is_timer_due(cntv_ctl: u64, cntv_cval: u64, cntpct: u64) -> bool {
    const ENABLE: u64 = 1 << 0;
    const IMASK:  u64 = 1 << 1;
    let enabled    = cntv_ctl & ENABLE != 0;
    let not_masked = cntv_ctl & IMASK  == 0;
    enabled && not_masked && cntpct >= cntv_cval
}

/// Inject PPI 27 into vcpu (vm_id, vcpu_id) to signal a virtual timer expiry.
///
/// The kernel sets HCR_EL2.VI on the next vcpu entry, causing the guest to take
/// a virtual IRQ at EL1. This is the "VI-bit injection" path (sufficient for P05;
/// proper GICH LR injection is P09).
pub fn inject_timer_irq(vm_id: usize, vcpu_id: usize) {
    #[cfg(target_arch = "aarch64")]
    {
        // sys_inject_irq(vm_id, vcpu_id, intid=27)
        unsafe {
            let mut ret: usize;
            core::arch::asm!(
                "svc #0",
                inlateout("x0") ViSyscall::InjectIrq as usize => ret,
                in("x1") vm_id,
                in("x2") vcpu_id,
                in("x3") VIRT_TIMER_PPI as usize,
                in("x4") 0usize,
                options(nostack, preserves_flags),
            );
            let _ = ret;
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (vm_id, vcpu_id);
        unimplemented!("inject_timer_irq: ARM64 only");
    }
}
