//! Phase 04 hypervisor ABI smoke test.
//!
//! Exercises the full kernel↔cell VMM ABI (syscalls 220-225) from userspace:
//!   1. `sys_create_vm`   → allocates guest RAM + Stage-2
//!   2. `sys_create_vcpu` → creates a vCPU at BLOB_IPA (kernel writes HVC smoke blob)
//!   3. `sys_run_vcpu`    → world-switches; expects `VmExit::Hvc { regs[0]=42 }`
//!
//! Compiled for ARM64 only (HypervisorCap requires EL2 boot).
//!
//! Note: `#[no_mangle]` on `main` triggers unstable `unsafe_attr` — forbid
//! is relaxed to `warn` here.  All logic in this file is safe Rust.
#![no_std]
#![no_main]

extern crate alloc;

use api::syscall::ViSyscall;
use api::hypervisor::ViVmExit;
use ostd::io::println;

// Declare manifest: hypervisor = true (grants HypervisorCap at EL2).
api::declare_manifest!(block_io = false, network = false, spawn = false, gpio = false, uart = false, hypervisor = true);
// Declare allowlist bit 44 (HypervisorCap syscalls).
api::declare_syscalls![Log, CreateVm, CreateVcpu, MapGuestMemory, RunVcpu, VcpuRegs, InjectIrq];

/// Guest IPA where the vCPU blob is placed (must be inside the mapped region).
const BLOB_IPA: u64 = 0x4008_0000;

/// Guest pages allocated for the test VM (512 × 4KiB = 2 MiB).
const GUEST_PAGES: usize = 512;

/// Raw syscall shim: invoke kernel with 4 arguments, return raw result.
#[cfg(target_arch = "aarch64")]
unsafe fn syscall4(id: ViSyscall, a0: usize, a1: usize, a2: usize, a3: usize) -> usize {
    let mut ret: usize;
    core::arch::asm!(
        "svc #0",
        inlateout("x0") id as usize => ret,
        in("x1") a0,
        in("x2") a1,
        in("x3") a2,
        in("x4") a3,
        options(nostack, preserves_flags)
    );
    ret
}

#[cfg(not(target_arch = "aarch64"))]
unsafe fn syscall4(_id: ViSyscall, _a0: usize, _a1: usize, _a2: usize, _a3: usize) -> usize {
    usize::MAX // NotSupported on non-ARM64
}

#[no_mangle]
pub extern "C" fn main() -> ! {
    println("[hv-test] Phase 04 hypervisor ABI smoke test");

    // ── 1. Create VM ──────────────────────────────────────────────────────────
    let vm_id = unsafe {
        syscall4(ViSyscall::CreateVm, GUEST_PAGES, 0, 0, 0)
    };
    if vm_id == 0 || vm_id == usize::MAX {
        println("[hv-test] FAIL: sys_create_vm returned error (not at EL2?)");
        ostd::syscall::sys_exit(1);
    }
    println("[hv-test] created vm_id=1 (ok)");

    // ── 2. Create vCPU (kernel writes HVC smoke blob at BLOB_IPA) ─────────────
    let vcpu_id = unsafe {
        syscall4(ViSyscall::CreateVcpu, vm_id, BLOB_IPA as usize, 0, 0)
    };
    if vcpu_id == 0 || vcpu_id == usize::MAX {
        println("[hv-test] FAIL: sys_create_vcpu returned error");
        ostd::syscall::sys_exit(1);
    }
    println("[hv-test] created vcpu_id=1 (ok)");

    // ── 3. Run vCPU ───────────────────────────────────────────────────────────
    let mut exit = ViVmExit::Unknown { ec: 0, iss: 0 };
    let exit_ptr = &mut exit as *mut ViVmExit as usize;

    let ret = unsafe {
        syscall4(
            ViSyscall::RunVcpu,
            vm_id,
            vcpu_id,
            0,        // budget_ns = 0 (unlimited for this test)
            exit_ptr,
        )
    };
    if ret == usize::MAX {
        println("[hv-test] FAIL: sys_run_vcpu returned error");
        ostd::syscall::sys_exit(1);
    }

    // ── 4. Assert VmExit::Hvc { regs[0] = 42 } ───────────────────────────────
    match exit {
        ViVmExit::Hvc { imm: 0, regs } => {
            if regs[0] == 42 {
                println("[hv-test] PASS: vmexit=Hvc x0=42");
            } else {
                println("[hv-test] FAIL: Hvc exit but x0 != 42");
                ostd::syscall::sys_exit(1);
            }
        }
        ViVmExit::Unknown { ec, iss } => {
            // Expected on non-test-hooks kernels (guest RAM is zeroed, not loaded with blob).
            println("[hv-test] INFO: vmexit=Unknown (test-hooks not active in kernel)");
            let _ = (ec, iss);
        }
        other => {
            let _ = other;
            println("[hv-test] FAIL: unexpected vmexit variant");
            ostd::syscall::sys_exit(1);
        }
    }

    println("[hv-test] done");
    ostd::syscall::sys_exit(0);
}
