//! Silo VmExit dispatch — handles only HVC and WFI exits.
//!
//! The silo guest is minimal: it boots, generates a P-256 key, and loops
//! waiting for mailbox requests signalled via an HVC. No MMIO, no GIC, no PL011.

extern crate alloc;

use api::hypervisor::ViVmExit;
use types::silo::{HVC_SILO_DONE, HVC_SILO_FAULT, HVC_SILO_READY};
use ostd::io::println;
use crate::vmm;

/// Result of running the silo guest for one mailbox operation.
pub enum SiloRunResult {
    /// Guest signalled HVC_SILO_READY or HVC_SILO_DONE. Mailbox response is valid.
    Done,
    /// Guest signalled HVC_SILO_FAULT. Byte arg = error code passed in x1.
    Fault(u8),
    /// Unrecoverable guest error — run_vcpu returned ERR or an unexpected exit.
    GuestError,
}

/// Run the silo guest until it signals HVC_SILO_READY, HVC_SILO_DONE, or HVC_SILO_FAULT.
///
/// WFI and Preempted exits are transparent — the vCPU is immediately re-entered.
/// Returns once the guest signals completion or on an unrecoverable error.
pub fn run_until_done(vm_id: usize, vcpu_id: usize) -> SiloRunResult {
    let mut exit = ViVmExit::Unknown { ec: 0, iss: 0 };
    loop {
        let ret = vmm::run_vcpu(vm_id, vcpu_id, &mut exit);
        if ret == usize::MAX {
            println("[silo] run_vcpu error — guest crashed");
            return SiloRunResult::GuestError;
        }
        match exit {
            ViVmExit::Hvc { imm: _, regs } => {
                let func_id = regs[0];
                if func_id == HVC_SILO_READY || func_id == HVC_SILO_DONE {
                    return SiloRunResult::Done;
                } else if func_id == HVC_SILO_FAULT {
                    // x1 carries the error code set by the guest before the HVC.
                    return SiloRunResult::Fault(regs[1] as u8);
                } else {
                    // Unknown HVC — log and re-enter so the guest can continue.
                    println(&alloc::format!("[silo] unknown HVC func_id=0x{:x}", func_id));
                }
            }
            ViVmExit::Wfi => {
                // Guest is idle (waiting for work after init) — re-enter immediately.
                continue;
            }
            ViVmExit::Preempted => {
                // Scheduler budget expired — re-enter (silo ops should finish promptly).
                continue;
            }
            ViVmExit::Unknown { ec, iss } => {
                println(&alloc::format!(
                    "[silo] unknown VmExit ec=0x{:x} iss=0x{:x} — aborting",
                    ec, iss
                ));
                return SiloRunResult::GuestError;
            }
            _ => {
                // MmioRead, MmioWrite, SysReg, Shutdown — none expected for the silo guest.
                println("[silo] unexpected VmExit variant — aborting");
                return SiloRunResult::GuestError;
            }
        }
    }
}
