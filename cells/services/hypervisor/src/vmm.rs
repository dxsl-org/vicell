//! Low-level VMM syscall wrappers (220-226) for the hypervisor service cell.
//!
//! These are ARM64-only at runtime (guarded by cpu_features::has_el2 at cell start).
//! RISC-V stubs return a NotSupported sentinel so the code compiles on all targets.

use api::syscall::ViSyscall;
use api::hypervisor::ViVmExit;

/// Scheduler tick budget for each RunVcpu call (~10ms in 10 MHz ticks = 100_000 ticks).
pub const SCHED_TICK_BUDGET_NS: u64 = 10_000_000; // 10ms in nanoseconds

/// Error sentinel returned by VMM syscalls on failure.
const ERR: usize = usize::MAX;

#[inline]
unsafe fn syscall4(id: ViSyscall, a0: usize, a1: usize, a2: usize, a3: usize) -> usize {
    let mut ret: usize;
    #[cfg(target_arch = "aarch64")]
    core::arch::asm!(
        "svc #0",
        inlateout("x0") id as usize => ret,
        in("x1") a0, in("x2") a1, in("x3") a2, in("x4") a3,
        options(nostack, preserves_flags),
    );
    #[cfg(not(target_arch = "aarch64"))]
    { let _ = (id, a0, a1, a2, a3); ret = ERR; }
    ret
}

/// Allocate guest RAM + Stage-2 table; returns vm_id (> 0) or 0 on error.
pub fn create_vm(guest_pages: usize) -> usize {
    unsafe { syscall4(ViSyscall::CreateVm, guest_pages, 0, 0, 0) }
}

/// Create a vCPU with initial PC `entry_pc` in `vm_id`; returns vcpu_id or 0.
pub fn create_vcpu(vm_id: usize, entry_pc: u64) -> usize {
    unsafe { syscall4(ViSyscall::CreateVcpu, vm_id, entry_pc as usize, 0, 0) }
}

/// Map guest IPA range in `vm_id`; returns 0 on success.
pub fn map_guest_memory(vm_id: usize, ipa: u64, size: usize, writable: bool) -> usize {
    unsafe { syscall4(ViSyscall::MapGuestMemory, vm_id, ipa as usize, size, writable as usize) }
}

/// Copy `src` bytes into guest RAM at `gpa`; returns bytes written or ERR.
pub fn write_guest_memory(vm_id: usize, gpa: u64, src: &[u8]) -> usize {
    unsafe {
        syscall4(
            ViSyscall::WriteGuestMemory,
            vm_id,
            gpa as usize,
            src.as_ptr() as usize,
            src.len(),
        )
    }
}

/// Copy `len` bytes from guest RAM at `gpa` into `dst`; returns bytes read or ERR.
pub fn read_guest_memory(vm_id: usize, gpa: u64, dst: &mut [u8]) -> usize {
    unsafe {
        syscall4(
            ViSyscall::ReadGuestMemory,
            vm_id,
            gpa as usize,
            dst.as_mut_ptr() as usize,
            dst.len(),
        )
    }
}

/// World-switch into `vcpu_id`; writes exit reason to `*exit`. Returns 0 or ERR.
pub fn run_vcpu(vm_id: usize, vcpu_id: usize, exit: &mut ViVmExit) -> usize {
    unsafe {
        syscall4(
            ViSyscall::RunVcpu,
            vm_id,
            vcpu_id,
            SCHED_TICK_BUDGET_NS as usize,
            exit as *mut ViVmExit as usize,
        )
    }
}

/// Read (write=false) or write (write=true) vCPU GP registers (32×u64 = x0..x30+pc).
pub fn vcpu_regs(vm_id: usize, vcpu_id: usize, regs: &mut [u64; 32], write: bool) -> usize {
    unsafe {
        syscall4(
            ViSyscall::VcpuRegs,
            vm_id,
            vcpu_id,
            regs.as_mut_ptr() as usize,
            write as usize,
        )
    }
}

/// Inject GICv2 virtual IRQ (0 ≤ intid ≤ 1019) into vCPU; returns 0 or ERR.
pub fn inject_irq(vm_id: usize, vcpu_id: usize, intid: u32) -> usize {
    unsafe { syscall4(ViSyscall::InjectIrq, vm_id, vcpu_id, intid as usize, 0) }
}
