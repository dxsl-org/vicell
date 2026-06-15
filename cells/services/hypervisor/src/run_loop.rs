//! VmExit dispatch loop.
//!
//! C2 (Red Team): run_vcpu uses a 10ms preempt budget so VFS/Net Cells stay live.
//! m2 (Red Team): default arm logs unregistered IPAs and continues (never silent).

extern crate alloc;

use api::hypervisor::ViVmExit;
use api::syscall::service;
use ostd::io::println;
use ostd::syscall::sys_lookup_service;
use crate::{
    gicd::Gicd, net_backend, pl011::Pl011, psci, timer, vmm,
    virtio_blk::BlkDisk,
    virtio_console::Console,
    virtio_mmio::{self, VirtioMmio},
    virtio_net::NetDev,
};

pub enum RunOutcome {
    Shutdown,
}

/// Main VMM run loop. Runs until the guest PSCI SYSTEM_OFF or an unrecoverable exit.
pub fn run(vm_id: usize, vcpu_id: usize) -> RunOutcome {
    // Resolve Net Cell TID for L2 frame bridging (0 = unavailable, bridging disabled).
    let net_tid = sys_lookup_service(service::NET).unwrap_or(0);

    let mut pl011    = Pl011::new();
    let mut gicd     = Gicd::new();
    let mut console  = Console::new();
    let mut vmio     = VirtioMmio::default();
    let mut blk      = BlkDisk::new();
    let mut blk_vmio = VirtioMmio::default();
    let mut net      = NetDev::new(net_tid);
    let mut net_vmio = VirtioMmio::default();
    let mut exit     = ViVmExit::Unknown { ec: 0, iss: 0 };

    loop {
        let ret = vmm::run_vcpu(vm_id, vcpu_id, &mut exit);
        if ret == usize::MAX {
            println("[hv] run_vcpu kernel error — aborting");
            return RunOutcome::Shutdown;
        }

        match exit {
            // ── HVC (PSCI + unknown) ──────────────────────────────────────────
            ViVmExit::Hvc { imm: 0, mut regs } => {
                match psci::dispatch(&mut regs) {
                    psci::PsciAction::Return(result) => {
                        let mut rb = [0u64; 32];
                        vmm::vcpu_regs(vm_id, vcpu_id, &mut rb, false);
                        rb[0] = result;
                        vmm::vcpu_regs(vm_id, vcpu_id, &mut rb, true);
                    }
                    psci::PsciAction::SystemOff | psci::PsciAction::SystemReset => {
                        println("[hv] PSCI SYSTEM_OFF");
                        return RunOutcome::Shutdown;
                    }
                }
            }
            ViVmExit::Hvc { imm, regs: _ } => {
                // Non-PSCI HVC — return NOT_SUPPORTED in x0 and advance past it.
                println(&alloc::format!("[hv] unknown HVC imm={}", imm));
                let mut rb = [0u64; 32];
                vmm::vcpu_regs(vm_id, vcpu_id, &mut rb, false);
                rb[0] = u64::MAX; // SMCCC NOT_SUPPORTED = -1
                vmm::vcpu_regs(vm_id, vcpu_id, &mut rb, true);
            }

            // ── MMIO write ───────────────────────────────────────────────────
            ViVmExit::MmioWrite { ipa, size, val } => {
                if Pl011::owns(ipa) {
                    pl011.write(ipa - crate::pl011::PL011_BASE_IPA, val);
                } else if Gicd::owns_gicd(ipa) {
                    gicd.write(ipa - crate::gicd::GICD_BASE_IPA, val, size);
                } else if Gicd::owns_gicc(ipa) {
                    // GICC writes: EOI / priority drop — safe to ignore with VI model.
                } else if virtio_mmio::owns(ipa) {
                    let (slot, off) = virtio_mmio::slot_and_offset(ipa);
                    match slot {
                        0 => vmio.mmio_write(off, val as u32, &mut console, vm_id, vcpu_id),
                        1 => blk_vmio.mmio_write(off, val as u32, &mut blk, vm_id, vcpu_id),
                        2 => net_vmio.mmio_write(off, val as u32, &mut net, vm_id, vcpu_id),
                        _ => {}
                    }
                } else {
                    println(&alloc::format!("[hv] unknown MMIO write ipa=0x{:x} val=0x{:x}", ipa, val));
                }
                advance_pc(vm_id, vcpu_id);
            }

            // ── MMIO read ────────────────────────────────────────────────────
            ViVmExit::MmioRead { ipa, size, reg } => {
                let val = if Pl011::owns(ipa) {
                    pl011.read(ipa - crate::pl011::PL011_BASE_IPA)
                } else if Gicd::owns_gicd(ipa) {
                    gicd.read(ipa - crate::gicd::GICD_BASE_IPA, size)
                } else if Gicd::owns_gicc(ipa) {
                    0u64
                } else if virtio_mmio::owns(ipa) {
                    let (slot, off) = virtio_mmio::slot_and_offset(ipa);
                    match slot {
                        0 => vmio.mmio_read(off, &console),
                        1 => blk_vmio.mmio_read(off, &blk),
                        2 => net_vmio.mmio_read(off, &net),
                        _ => 0,
                    }
                } else {
                    println(&alloc::format!("[hv] unknown MMIO read ipa=0x{:x}", ipa));
                    0u64
                };
                let mut rb = [0u64; 32];
                vmm::vcpu_regs(vm_id, vcpu_id, &mut rb, false);
                if (reg as usize) < 31 {
                    rb[reg as usize] = val;
                }
                vmm::vcpu_regs(vm_id, vcpu_id, &mut rb, true);
                advance_pc(vm_id, vcpu_id);
            }

            // ── WFI — inject virtual timer; poll for guest RX frames ─────────
            ViVmExit::Wfi => {
                timer::inject_timer_irq(vm_id, vcpu_id);
                if let Some(frame) = net_backend::try_receive(net_tid) {
                    net.push_rx_frame(&frame, vm_id, vcpu_id, &net_vmio);
                }
            }

            // ── Preemption budget expired (C2 yield) — poll RX before re-enter
            ViVmExit::Preempted => {
                if let Some(frame) = net_backend::try_receive(net_tid) {
                    net.push_rx_frame(&frame, vm_id, vcpu_id, &net_vmio);
                }
            }

            // ── Guest shutdown ────────────────────────────────────────────────
            ViVmExit::Shutdown => {
                println("[hv] ViVmExit::Shutdown");
                return RunOutcome::Shutdown;
            }

            // ── Sysreg trap — return 0 for reads, ignore writes ──────────────
            ViVmExit::SysReg { rt, is_write, .. } => {
                if !is_write && (rt as usize) < 31 {
                    let mut rb = [0u64; 32];
                    vmm::vcpu_regs(vm_id, vcpu_id, &mut rb, false);
                    rb[rt as usize] = 0;
                    vmm::vcpu_regs(vm_id, vcpu_id, &mut rb, true);
                }
                advance_pc(vm_id, vcpu_id);
            }

            // ── Unknown exit ─────────────────────────────────────────────────
            ViVmExit::Unknown { ec, iss } => {
                println(&alloc::format!("[hv] unknown vmexit ec=0x{:x} iss=0x{:x}", ec, iss));
                return RunOutcome::Shutdown;
            }
        }
    }
}

/// Advance guest PC by 4 bytes past the trapped instruction.
/// reg_buf layout: x0..x30 at [0..30], PC at [31].
fn advance_pc(vm_id: usize, vcpu_id: usize) {
    let mut rb = [0u64; 32];
    vmm::vcpu_regs(vm_id, vcpu_id, &mut rb, false);
    rb[31] = rb[31].wrapping_add(4);
    vmm::vcpu_regs(vm_id, vcpu_id, &mut rb, true);
}
