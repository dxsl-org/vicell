//! Kernel event waker — IRQ-driven wake for WaitForEvent syscall.
//!
//! Architecture: a single `NET_RX_PENDING` AtomicBool is set by the VirtIO NIC
//! ISR on every RX interrupt.  The global timer sweep (hart 0, `pick_next`) checks
//! all `WaitEvent` tasks each tick and wakes those whose mask matches a pending bit.
//! The net cell checks the flag BEFORE parking (lost-wakeup guard): if NET_RX_PENDING
//! is already set when WaitForEvent is called, it returns immediately without blocking.
//!
//! Lock order: SCHEDULER (global) → per-hart ready (leaf).
//! This module does NOT acquire SCHEDULER itself — callers in the sweep already hold it.

use core::sync::atomic::{AtomicBool, Ordering};

/// Set by the VirtIO NIC ISR whenever at least one RX frame is available.
/// Cleared by the WaitForEvent handler when the net cell is woken.
pub static NET_RX_PENDING: AtomicBool = AtomicBool::new(false);

/// Signal a NIC RX event.  Called from the VirtIO interrupt handler (ISR context).
///
/// Sets `NET_RX_PENDING` so the next timer sweep wakes any `WaitEvent(NET_RX)` task.
/// On RISC-V we also pend local SSIP so the timer handler fires without waiting for
/// the next mtime tick — sub-millisecond latency on the handling hart.
///
/// # Safety contract
/// Callers must be in S-mode trap context (SIE already cleared by hardware entry).
pub fn signal_net_rx() {
    NET_RX_PENDING.store(true, Ordering::Release);
    // Pend a software interrupt on the current hart so vi_timer_tick fires immediately.
    // SAFETY: csrsi sip.SSIP is permitted from S-mode (RISC-V priv spec §4.1.3).
    // SIE is currently cleared by the hardware trap entry, so this is queued and
    // fires once the ISR returns and sret restores sstatus.SIE.
    #[cfg(target_arch = "riscv64")]
    unsafe { core::arch::asm!("csrsi sip, 0x2", options(nomem, nostack)) };
}

/// Check whether event `mask` has any pending bits.  Returns the matching fired bits,
/// or 0 if none.  Clears the matching bits as a side effect (consume-on-read).
///
/// Called by the timer sweep (already under SCHEDULER) and the WaitForEvent syscall
/// handler before parking the task.
pub fn consume_pending(mask: u32) -> u32 {
    let mut fired: u32 = 0;
    if mask & api::syscall::events::NET_RX != 0
        && NET_RX_PENDING.compare_exchange(true, false, Ordering::AcqRel, Ordering::Relaxed).is_ok()
    {
        fired |= api::syscall::events::NET_RX;
    }
    fired
}
