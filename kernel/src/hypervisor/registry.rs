//! VM registry — per-owner RAII store for Stage-2 tables, guest RAM, and vCPUs.
//!
//! # Lock order
//! `VM_REGISTRY` → `FRAME_ALLOCATOR` (same order as grant reaper; never reverse).
//!
//! # Non-aarch64
//! All public functions return `Err(ViError::NotSupported)` so the compiler
//! produces a complete match for the hypervisor Syscall arms on every target.

extern crate alloc;
use alloc::{collections::BTreeMap, vec::Vec};
use types::{ViError, ViResult};
use crate::sync::Spinlock;

// ── AArch64-only concrete types ───────────────────────────────────────────────

#[cfg(target_arch = "aarch64")]
use crate::memory::stage2::Stage2Table;
#[cfg(target_arch = "aarch64")]
use hal::aarch64::{stage2_regs::{enable_stage2, disable_stage2}, vcpu::{AArch64Vcpu, run_vcpu_impl}};
#[cfg(target_arch = "aarch64")]
use api::hypervisor::ViVmExit as ApiVmExit;
#[cfg(target_arch = "aarch64")]
use hal::ViVmExit as HalVmExit;

// ── VM entry ──────────────────────────────────────────────────────────────────

#[cfg(target_arch = "aarch64")]
struct Vm {
    stage2:  Stage2Table,
    guest_pa: u64,
    guest_pages: usize,
    vcpus:   Vec<AArch64Vcpu>,
    vmid:    u16,
}

// VM_REGISTRY is keyed by (owner_tid, vm_id).
// vm_id is assigned sequentially per owner; starts at 1.
#[cfg(target_arch = "aarch64")]
static VM_REGISTRY: Spinlock<Option<BTreeMap<(usize, usize), Vm>>> = Spinlock::new(None);

#[cfg(target_arch = "aarch64")]
static NEXT_VMID: core::sync::atomic::AtomicU16 = core::sync::atomic::AtomicU16::new(1);

#[cfg(target_arch = "aarch64")]
fn registry_lock() -> &'static Spinlock<Option<BTreeMap<(usize, usize), Vm>>> {
    &VM_REGISTRY
}

// ── Sequential vm_id counter per owner ───────────────────────────────────────

/// Per-owner sequential VM-id counter, stored alongside each owner's first VM.
/// Simple: we just use the total registered VM count + 1 as the next id.
#[cfg(target_arch = "aarch64")]
fn next_vm_id_for(owner: usize) -> usize {
    let guard = registry_lock().lock();
    let count = guard.as_ref().map_or(0, |m| {
        m.keys().filter(|(o, _)| *o == owner).count()
    });
    count + 1
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Allocate guest RAM + Stage-2 table; return opaque `vm_id`.
pub fn create_vm(owner: usize, guest_pages: usize) -> ViResult<usize> {
    #[cfg(target_arch = "aarch64")]
    {
        use crate::memory::paging::PAGE_SIZE;

        let mut table = Stage2Table::new().ok_or(ViError::OutOfMemory)?;
        let guest_pa  = table.carve_guest_ram(guest_pages).ok_or(ViError::OutOfMemory)?;
        // Map all guest RAM at IPA 0x40000000.
        table.map(0x4000_0000, guest_pa, guest_pages, true)
             .map_err(|_| ViError::OutOfMemory)?;

        let vmid = NEXT_VMID.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        // SAFETY: table built and flushed; vmid ≥ 1; not yet active (enable later).
        unsafe { enable_stage2(vmid, table.root_pa()); }

        let vm_id = {
            let mut guard = registry_lock().lock();
            if guard.is_none() { *guard = Some(BTreeMap::new()); }
            let map = guard.as_mut().unwrap();
            let id = map.keys().filter(|(o, _)| *o == owner).count() + 1;
            map.insert((owner, id), Vm { stage2: table, guest_pa, guest_pages, vcpus: Vec::new(), vmid });
            let _ = PAGE_SIZE; // suppress unused warning
            id
        };
        Ok(vm_id)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (owner, guest_pages);
        Err(ViError::NotSupported)
    }
}

/// Create a vCPU in `vm_id` with initial PC `entry_pc`; return `vcpu_id` (1-based).
///
/// Under `test-hooks`, writes a P04 HVC smoke blob (`MOVZ X0,#42; HVC #0; B .`)
/// to the page containing `entry_pc` so the test cell does not need userspace
/// memory access to guest RAM.
pub fn create_vcpu(owner: usize, vm_id: usize, entry_pc: u64) -> ViResult<usize> {
    #[cfg(target_arch = "aarch64")]
    {
        let mut guard = registry_lock().lock();
        let map = guard.as_mut().ok_or(ViError::NotFound)?;
        let vm = map.get_mut(&(owner, vm_id)).ok_or(ViError::NotFound)?;
        let vcpu_id = vm.vcpus.len() + 1;

        // test-hooks: write P04 HVC smoke blob so the test cell can verify Hvc exit.
        #[cfg(feature = "test-hooks")]
        {
            const MOVZ_X0_42: u32 = 0xD280_0540; // MOVZ X0, #42
            const HVC_0:      u32 = 0xD400_0002; // HVC #0
            const B_DOT:      u32 = 0x1400_0000; // B .
            const GUEST_IPA_BASE: u64 = 0x4000_0000;
            let offset = (entry_pc - GUEST_IPA_BASE) as usize;
            let blob_pa = vm.guest_pa as usize + offset;
            // SAFETY: guest RAM is kernel-allocated identity-mapped memory; no active vCPU yet.
            unsafe {
                let ptr = blob_pa as *mut u32;
                ptr.write(MOVZ_X0_42);
                ptr.add(1).write(HVC_0);
                ptr.add(2).write(B_DOT);
            }
        }

        vm.vcpus.push(AArch64Vcpu::new(entry_pc));
        Ok(vcpu_id)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (owner, vm_id, entry_pc);
        Err(ViError::NotSupported)
    }
}

/// Map guest IPA range in `vm_id`'s Stage-2.
pub fn map_guest_memory(owner: usize, vm_id: usize, ipa: u64, size: usize, writable: bool) -> ViResult<()> {
    #[cfg(target_arch = "aarch64")]
    {
        use crate::memory::paging::PAGE_SIZE;
        let pages = size.div_ceil(PAGE_SIZE);
        let mut guard = registry_lock().lock();
        let map = guard.as_mut().ok_or(ViError::NotFound)?;
        let vm = map.get_mut(&(owner, vm_id)).ok_or(ViError::NotFound)?;
        // Extend guest RAM mapping to cover the requested IPA range.
        vm.stage2.map(ipa, vm.guest_pa, pages, writable)
            .map_err(|_| ViError::OutOfMemory)?;
        Ok(())
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (owner, vm_id, ipa, size, writable);
        Err(ViError::NotSupported)
    }
}

/// World-switch into vCPU; write `ViVmExit` to `exit_out`.
///
/// # Safety
/// `exit_out` must point to a valid, writable `ViVmExit`-sized buffer in the
/// caller's address space.  Validated by the syscall layer before this call.
pub fn run_vcpu(owner: usize, vm_id: usize, vcpu_id: usize, _budget_ns: u64,
                exit_out: *mut api::hypervisor::ViVmExit) -> ViResult<usize> {
    #[cfg(target_arch = "aarch64")]
    {
        let hal_exit = {
            let mut guard = registry_lock().lock();
            let map = guard.as_mut().ok_or(ViError::NotFound)?;
            let vm = map.get_mut(&(owner, vm_id)).ok_or(ViError::NotFound)?;
            let vcpu = vm.vcpus.get_mut(vcpu_id.saturating_sub(1)).ok_or(ViError::NotFound)?;
            // SAFETY: Stage-2 is enabled for this VMID; vcpu exclusively owned under lock.
            unsafe { run_vcpu_impl(vcpu) }
        };

        // Convert HAL ViVmExit → API ViVmExit (same fields, different crate paths).
        let api_exit = match hal_exit {
            HalVmExit::MmioRead  { ipa, size, reg } => ApiVmExit::MmioRead  { ipa, size, reg },
            HalVmExit::MmioWrite { ipa, size, val } => ApiVmExit::MmioWrite { ipa, size, val },
            HalVmExit::Hvc      { imm, regs }       => ApiVmExit::Hvc      { imm, regs },
            HalVmExit::Wfi                           => ApiVmExit::Wfi,
            HalVmExit::SysReg   { op0, op1, crn, crm, op2, rt, is_write }
                                                     => ApiVmExit::SysReg  { op0, op1, crn, crm, op2, rt, is_write },
            HalVmExit::Preempted                     => ApiVmExit::Preempted,
            HalVmExit::Shutdown                      => ApiVmExit::Shutdown,
            HalVmExit::Unknown  { ec, iss }          => ApiVmExit::Unknown  { ec, iss },
        };
        // SAFETY: exit_out validated by syscall layer.
        unsafe { core::ptr::write(exit_out, api_exit); }
        Ok(0)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (owner, vm_id, vcpu_id, _budget_ns, exit_out);
        Err(ViError::NotSupported)
    }
}

/// Read or write vCPU general-purpose registers (x0-x30 + sp + pc = 32×u64).
pub fn vcpu_regs(owner: usize, vm_id: usize, vcpu_id: usize, buf_ptr: usize, write: bool) -> ViResult<usize> {
    #[cfg(target_arch = "aarch64")]
    {
        let mut guard = registry_lock().lock();
        let map = guard.as_mut().ok_or(ViError::NotFound)?;
        let vm = map.get_mut(&(owner, vm_id)).ok_or(ViError::NotFound)?;
        let vcpu = vm.vcpus.get_mut(vcpu_id.saturating_sub(1)).ok_or(ViError::NotFound)?;
        // buf_ptr points to 32×u64 (256 bytes), validated by syscall layer.
        // SAFETY: buf_ptr validated; SAS — same VA in kernel and cell.
        let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u64, 32) };
        if write {
            // Write x0-x30 from buf[0..31]; buf[31] = pc (g_elr_el2).
            for (i, v) in buf[..31].iter().enumerate() { vcpu.gp[i] = *v; }
            vcpu.g_elr_el2 = buf[31];
        } else {
            for (i, v) in vcpu.gp.iter().enumerate() { buf[i] = *v; }
            buf[31] = vcpu.g_elr_el2;
        }
        Ok(0)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (owner, vm_id, vcpu_id, buf_ptr, write);
        Err(ViError::NotSupported)
    }
}

/// Copy `len` bytes from caller's `src_ptr` into guest physical RAM at `gpa`.
///
/// # Preconditions (enforced by caller / syscall layer)
/// - `src_ptr + len` is within the caller cell's valid address range (via `validate_user_buf`).
/// - `gpa + len` does not wrap (overflow guard in syscall layer).
///
/// # Safety (kernel-internal)
/// `src_ptr` is a valid cell VA; in SAS, VA == PA for kernel-managed regions, but
/// the copy uses `copy_nonoverlapping` which only reads the source — no guest access.
pub fn write_guest_memory(owner: usize, vm_id: usize, gpa: u64, src_ptr: usize, len: usize) -> ViResult<usize> {
    #[cfg(target_arch = "aarch64")]
    {
        use crate::memory::paging::PAGE_SIZE;
        const GUEST_IPA_BASE: u64 = 0x4000_0000;

        let guard = registry_lock().lock();
        let map = guard.as_ref().ok_or(ViError::NotFound)?;
        let vm  = map.get(&(owner, vm_id)).ok_or(ViError::NotFound)?;

        // Validate gpa is within the mapped guest-RAM window.
        let offset = gpa.checked_sub(GUEST_IPA_BASE)
            .ok_or(ViError::InvalidInput)? as usize;
        let end = offset.checked_add(len).ok_or(ViError::InvalidInput)?;
        if end > vm.guest_pages * PAGE_SIZE {
            return Err(ViError::InvalidInput);
        }

        // SAFETY: guest RAM is kernel-allocated identity-mapped memory.
        // src_ptr validated by syscall layer (validate_user_buf); SAS means it's
        // also accessible here. No active vCPU reads this region while we copy
        // (the caller holds no vcpu run in progress — that would require RunVcpu,
        // which cannot be concurrent in a single-task cell).
        unsafe {
            let dst = (vm.guest_pa as usize + offset) as *mut u8;
            let src = src_ptr as *const u8;
            core::ptr::copy_nonoverlapping(src, dst, len);
        }
        Ok(len)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (owner, vm_id, gpa, src_ptr, len);
        Err(ViError::NotSupported)
    }
}

/// Copy `len` bytes from guest physical RAM at `gpa` into caller's `dst_ptr`.
///
/// # Preconditions (enforced by caller / syscall layer)
/// - `dst_ptr + len` is within the caller cell's valid address range (via `validate_user_buf`).
/// - `gpa + len` does not wrap (overflow guard in syscall layer).
///
/// # Safety (kernel-internal)
/// `dst_ptr` is a valid cell VA; in SAS, VA == PA for kernel-managed regions.
/// Guest RAM is kernel-allocated identity-mapped memory — never freed while a vCPU
/// is alive (teardown requires the VM to be destroyed first).
pub fn read_guest_memory(owner: usize, vm_id: usize, gpa: u64, dst_ptr: usize, len: usize) -> ViResult<usize> {
    #[cfg(target_arch = "aarch64")]
    {
        use crate::memory::paging::PAGE_SIZE;
        const GUEST_IPA_BASE: u64 = 0x4000_0000;

        let guard = registry_lock().lock();
        let map = guard.as_ref().ok_or(ViError::NotFound)?;
        let vm  = map.get(&(owner, vm_id)).ok_or(ViError::NotFound)?;

        // Validate gpa is within the mapped guest-RAM window.
        let offset = gpa.checked_sub(GUEST_IPA_BASE)
            .ok_or(ViError::InvalidInput)? as usize;
        let end = offset.checked_add(len).ok_or(ViError::InvalidInput)?;
        if end > vm.guest_pages * PAGE_SIZE {
            return Err(ViError::InvalidInput);
        }

        // SAFETY: guest RAM is kernel-allocated identity-mapped memory.
        // dst_ptr validated by syscall layer (validate_user_buf); SAS means it's
        // also accessible here. No active vCPU writes this region while we copy
        // (the caller holds no vcpu run in progress — that would require RunVcpu,
        // which cannot be concurrent in a single-task cell).
        unsafe {
            let src = (vm.guest_pa as usize + offset) as *const u8;
            let dst = dst_ptr as *mut u8;
            core::ptr::copy_nonoverlapping(src, dst, len);
        }
        Ok(len)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (owner, vm_id, gpa, dst_ptr, len);
        Err(ViError::NotSupported)
    }
}

/// Inject GICv2 virtual interrupt into vCPU (intid ≤ 1019, validated by caller).
pub fn inject_irq(owner: usize, vm_id: usize, vcpu_id: usize, intid: u32) -> ViResult<usize> {
    // P05+ will write to GICH_LR registers here.  For P04 stub, record in vcpu state.
    let _ = (owner, vm_id, vcpu_id, intid);
    // TODO(P05): route intid through GICH LR via HCR_EL2.VI or LR injection.
    Ok(0)
}

// ── Teardown — called on every task-exit path ─────────────────────────────────

/// Reclaim all VMs and guest RAM owned by `dead_tid`.
///
/// Called alongside `reap_grants_for_task` on task exit, fault, and watchdog kill.
/// Lock order: VM_REGISTRY → FRAME_ALLOCATOR (same as grant reaper).
pub fn reap_vms_for_task(dead_tid: usize) {
    #[cfg(target_arch = "aarch64")]
    {
        // Collect entries to drop outside the lock (Stage2Table::drop frees frames).
        let dead_vms: alloc::vec::Vec<Vm> = {
            let mut guard = registry_lock().lock();
            let Some(map) = guard.as_mut() else { return };
            let dead_keys: alloc::vec::Vec<(usize, usize)> = map
                .keys()
                .filter(|(o, _)| *o == dead_tid)
                .copied()
                .collect();
            dead_keys.iter().filter_map(|k| map.remove(k)).collect()
        };
        // Disable Stage-2 for each dying VM before dropping the table.
        for vm in dead_vms {
            // SAFETY: no vCPU is running (task is dead); safe to disable Stage-2.
            unsafe { disable_stage2(); }
            drop(vm); // Stage2Table::drop frees all frames
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = dead_tid;
    }
}
