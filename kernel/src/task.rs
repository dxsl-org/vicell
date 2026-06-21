pub mod cap;
pub mod hart_local;
pub mod smp;
pub mod syscall;
pub mod tcb;
pub use tcb::Task;
pub mod drivers;
pub mod ipc_test;
pub mod scheduler;
pub mod stack;
pub mod user_hello;
pub mod waker;

#[cfg(test)]
mod tests;

use crate::sync::Spinlock;
use alloc::string::String;
use log::info;
use scheduler::Scheduler;
/// Increased to 64 (256 KB): fatfs nests deep call frames during recursive
/// directory removal; 16 pages (64 KB) overflows on complex FAT16 ops.
pub const STACK_PAGES: usize = 64;
const TRAP_FRAME_SIZE: usize = core::mem::size_of::<crate::hal::arch::ViTrapFrame>();
extern "C" {
    fn __trap_exit();
}

// use alloc::vec::Vec;
use tcb::{SyscallFuture, TaskState};
use types::*;

// Global Scheduler Instance
pub(crate) static SCHEDULER: Spinlock<Option<Scheduler>> = Spinlock::new(None);

// Global Tick Counter
static TICKS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

// Helper context to save the initial boot/kernel state during first task switch
#[cfg(target_arch = "riscv64")]
static mut BOOT_CONTEXT: crate::hal::arch::Context = crate::hal::arch::Context {
    ra: 0, sp: 0, s0: 0, s1: 0, s2: 0, s3: 0, s4: 0, s5: 0,
    s6: 0, s7: 0, s8: 0, s9: 0, s10: 0, s11: 0,
    sepc: 0, sstatus: 0x102, gp: 0, tp: 0, sscratch: 0,
};
#[cfg(target_arch = "aarch64")]
static mut BOOT_CONTEXT: crate::hal::arch::Context = crate::hal::arch::Context {
    x19: 0, x20: 0, x21: 0, x22: 0, x23: 0, x24: 0, x25: 0,
    x26: 0, x27: 0, x28: 0, x29: 0, x30: 0, sp: 0,
    elr_el1: 0, spsr_el1: 0x305, sp_el0: 0,
    daif: 0, // saved/restored by __switch_el1; 0 = no DAIF masking (IRQs enabled)
};
#[cfg(target_arch = "riscv32")]
static mut BOOT_CONTEXT: crate::hal::arch::Context = crate::hal::arch::Context {
    ra: 0, sp: 0, s0: 0, s1: 0, s2: 0, s3: 0, s4: 0, s5: 0,
    s6: 0, s7: 0, s8: 0, s9: 0, s10: 0, s11: 0,
    sepc: 0, sstatus: 0x102, gp: 0, tp: 0, sscratch: 0,
};
#[cfg(target_arch = "x86_64")]
static mut BOOT_CONTEXT: crate::hal::arch::Context = crate::hal::arch::Context {
    r15: 0, r14: 0, r13: 0, r12: 0, rbx: 0, rbp: 0, sp: 0, rip: 0, kernel_trap_sp: 0,
};
#[cfg(target_arch = "arm")]
static mut BOOT_CONTEXT: crate::hal::arch::Context = crate::hal::arch::Context {
    r4: 0, r5: 0, r6: 0, r7: 0, r8: 0, r9: 0, r10: 0, r11: 0, sp: 0, lr: 0, cpsr: 0x13,
};
#[cfg(target_arch = "x86")]
static mut BOOT_CONTEXT: crate::hal::arch::Context = crate::hal::arch::Context {
    ebx: 0, esi: 0, edi: 0, ebp: 0, sp: 0, eip: 0,
};

// Trampoline for Thread Spawning
// Trampoline for Thread Spawning handled by HAL

extern "C" {
    // pub fn thread_trampoline(); // In HAL
}

pub fn get_kernel_gp_tp() -> (usize, usize) {
    crate::hal::arch::get_gp_tp()
}

pub fn system_ticks() -> usize {
    TICKS.load(core::sync::atomic::Ordering::Relaxed)
}

pub fn tick() {
    TICKS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}

pub fn init() {
    // Install hart-local state for hart 0 BEFORE the scheduler or timer start,
    // so current_cell_id() works correctly once interrupts are enabled.
    hart_local::install(0);

    info!("Process: Initializing Scheduler...");
    let mut sched_guard = SCHEDULER.lock();

    // SAFETY: Use ptr::write to overwrite the Spinlock guard's data WITHOUT dropping the old value.
    // This prevents "Freed node aliases existing hole" panic on soft reboot (where .data persists but Heap is reset).
    unsafe {
        core::ptr::write(&mut *sched_guard, Some(Scheduler::new()));
    }
    drop(sched_guard);

    // Enable S-mode timer interrupt and arm the first preemption tick.
    // Done after scheduler init so vi_timer_tick() sees a valid SCHEDULER.
    #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
    {
        // SAFETY: sets STIE (bit 5 = mask 0x20) in sie from S-mode. Must use the
        // register form of csrs — csrsi's immediate is only 5 bits (0..=31), so a
        // 0x20 mask cannot be encoded as an immediate.
        unsafe { core::arch::asm!("csrs sie, {stie}", stie = in(reg) 0x20usize); }
        let next = hal::common::timer::read_mtime() + hal::common::timer::TICKS_PER_10MS;
        hal::common::sbi::set_timer(next);
        info!("Timer preemption enabled (10 ms timeslice)");
    }
}


/// Exposes `terminate_current_cell_on_fault` to the HAL trap handler via
/// `extern "Rust"` linkage.
#[no_mangle]
pub extern "Rust" fn vi_terminate_on_fault(scause: usize, sepc: usize, stval: usize) {
    terminate_current_cell_on_fault(scause, sepc, stval);
}

/// Exposes `scheduler::current_cell_id` to the HAL trap handler.
#[no_mangle]
pub extern "Rust" fn vi_current_cell_id() -> usize {
    scheduler::current_cell_id()
}

/// Called from the S-mode timer ISR via `extern "Rust"` linkage.
///
/// Increments the global tick counter, rearmed the timer for the next
/// 10 ms slice, and yields the CPU so the scheduler can preempt the
/// current task if a higher-priority task has become runnable.
#[no_mangle]
pub extern "Rust" fn vi_timer_tick() {
    tick();

    // Rearm timer anchored to current mtime so the slice is constant
    // regardless of how long this ISR takes.
    #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
    {
        let next = hal::common::timer::read_mtime() + hal::common::timer::TICKS_PER_10MS;
        hal::common::sbi::set_timer(next);
    }

    // Poll VirtIO input on every 10 ms tick.
    //
    // VirtIO RING_EVENT_IDX may suppress the device→driver interrupt even when
    // QEMU places an event in the used ring.  Polling the ring directly ensures
    // keyboard/mouse events are never dropped regardless of IRQ delivery.
    // ipc_send(0, …) is fire-and-forget (caller_id=0 has no task entry), so
    // this is safe to call from timer-ISR context with interrupts disabled.
    crate::task::drivers::virtio_input::poll_events();
    crate::task::drivers::virtio_input::dispatch_pending();

    // Poll UART hardware and relay any new bytes to the input service.
    // Makes UART delivery reader-independent: events arrive even when no cell
    // is currently blocked in sys_read(0).  VirtIO events were already drained
    // above, so the VirtIO section of poll() is a no-op here.
    crate::task::drivers::console_drv::CONSOLE.lock().poll();

    // Run the scheduler.  If a higher-priority (or simply next round-robin)
    // task is ready, this performs a context switch.  Safe to call from the
    // timer ISR because:
    //   (a) interrupts are disabled by hardware on trap entry (sstatus.SIE=0)
    //   (b) yield_cpu() releases SCHEDULER lock before calling Context::switch
    //   (c) trap.S restores the correct ViTrapFrame from the new task's stack
    yield_cpu();
}

/// Force-release every global kernel Spinlock during fault teardown.
///
/// A Cell holds no kernel lock legitimately, so on a genuine U-mode Cell fault
/// these are all free.  The real hazard is a kernel `panic!`/`expect()` raised
/// *while servicing a Cell's syscall* (`CURRENT_CELL_ID != 0`) that was holding
/// one of these — without releasing it the lock stays held forever and the next
/// acquirer deadlocks, hanging the whole kernel.
///
/// # Safety
/// Single-hart kernel, called only from the fault/panic teardown path with
/// interrupts disabled.  No other context can hold these; force-unlocking an
/// already-free Spinlock is a no-op.
///
/// NOTE: the linked_list_allocator heap lock is intentionally not covered (no
/// external force-unlock API); a panic *inside* the allocator implies heap
/// corruption — a reboot-class fault, not a recoverable Cell kill.
unsafe fn force_unlock_all_kernel_locks() {
    SCHEDULER.force_unlock();
    crate::memory::frame::FRAME_ALLOCATOR.force_unlock();
    crate::cell::registry::CELL_REGISTRY.force_unlock();
    crate::cell::cap_registry::CAP_TABLE.force_unlock();
    crate::memory::cell_quota::force_unlock_locks();
    crate::memory::rt_heap::force_unlock_locks();
    crate::cell::hotswap::force_unlock_locks();
    crate::cell::service_registry::force_unlock_locks();
    crate::task::drivers::virtio_blk::force_unlock_locks();
    crate::task::drivers::virtio_input::force_unlock_locks();
    crate::task::drivers::mmc::force_unlock_locks();
    crate::task::drivers::blk_nvme::force_unlock_locks();
    crate::resource_registry::force_unlock_locks();
    crate::measurement_log::force_unlock_locks();
}

/// Terminate the currently-executing Cell due to a hardware fault.
///
/// Called from the trap handler when an unrecoverable exception fires in a
/// Cell context (`scheduler::CURRENT_CELL_ID != 0`).  The Cell moves to the
/// zombie list; the kernel continues running the next ready task.
///
/// # Safety
/// Must be called from trap context with S-mode interrupts disabled.
/// Force-unlocks ALL global kernel locks first (see
/// [`force_unlock_all_kernel_locks`]) so a panic that fired while holding one
/// (e.g. mid-syscall OOM during a scheduler/allocator insert) cannot deadlock
/// the kernel after we resume the next task.
pub fn terminate_current_cell_on_fault(scause: usize, sepc: usize, stval: usize) {
    let cell_id_raw = hart_local::current_cell_id();
    log::error!(
        "[fault] Cell {} terminated: scause={:#x} sepc={:#x} stval={:#x}",
        cell_id_raw, scause, sepc, stval
    );
    crate::audit::log_event(
        crate::audit::AuditEvent::CellFault,
        &crate::audit::encode_u32x2(cell_id_raw as u32, scause as u32),
    );

    // The panic/fault may have fired while kernel code (servicing this Cell's
    // syscall) held one or more global locks.  Release ALL of them before we
    // re-acquire anything, else the next acquirer deadlocks forever.
    // SAFETY: single-hart kernel, interrupts disabled here; no other context
    // holds these, and force-unlocking a free lock is a no-op.
    unsafe { force_unlock_all_kernel_locks(); }

    let current_tid = hart_local::ready::current_task_id_for(hart_local::current_hart_id());
    let task_id = if current_tid > 0 { Some(current_tid) } else { None };

    if let Some(tid) = task_id {
        if let Some(sched) = SCHEDULER.lock().as_mut() {
            // usize::MAX = fault reason; also wakes any Wait(tid) waiters.
            sched.exit_task(tid, usize::MAX);
        }
        // Deregister quota and MMIO regions for the killed Cell.
        let cell_id = types::CellId(cell_id_raw as u64);
        crate::memory::cell_quota::deregister(cell_id);
        crate::resource_registry::release_for(cell_id);
        // Reap grant/registered-grant pages for this cell. SCHEDULER lock is
        // already released (the block above exited), so the
        // KERNEL_ROOT → FRAME_ALLOCATOR path inside reap_grants_for_task is safe.
        crate::task::syscall::reap_grants_for_task(tid);
        // Reap any VMs (guest RAM + Stage-2 tables) owned by this cell.
        crate::hypervisor::registry::reap_vms_for_task(tid);
    }

    // If the faulting cell owned the fast-IPC VFS handler, null the pointer so
    // future call_vfs() invocations don't jump into dead/replaced cell state.
    crate::fast_ipc::clear_vfs_if_cell(cell_id_raw);

    // Reset cell ID to 0 (kernel context) so subsequent allocations are not
    // charged to the now-dead Cell.
    hart_local::set_current_cell_id(0);

    // Switch to the next ready task.  Does not return to the faulting Cell.
    yield_cpu();
}

/// Core scheduling logic: picks next task and performs switch OUTSIDE of the lock.
pub fn yield_cpu() {
    // x86_64: disable interrupts for the entire scheduler critical section.
    // Without this, the LAPIC timer fires mid-lock and causes a nested
    // yield_cpu call that deadlocks on the same spinlock (IRQ-in-lock deadlock).
    // RISC-V/AArch64 automatically clear the interrupt-enable bit on trap entry,
    // so they don't have this problem when called from vi_timer_tick.
    #[cfg(target_arch = "x86_64")]
    unsafe { core::arch::asm!("cli", options(nomem, nostack)); }

    // Reap zombies already switched away from. Take them under the lock (cheap
    // pointer moves), then drop OUTSIDE it so Stack::drop's frame-free + unmap
    // (FRAME_ALLOCATOR / KERNEL_ROOT) never run while SCHEDULER is held. This is
    // what frees a dead cell's stacks — without it every cell death leaked them
    // (e.g. the shell-supervisor restart loop would grow until OOM).
    let reaped = {
        if let Some(sched) = SCHEDULER.lock().as_mut() {
            sched.take_reapable_zombies()
        } else {
            alloc::vec::Vec::new()
        }
    };
    drop(reaped);

    // Reap grant pages for watchdog-killed tasks. The watchdog enqueues task IDs
    // here because free_grant_pages (KERNEL_ROOT → FRAME_ALLOCATOR) must not run
    // while SCHEDULER is held — same pattern as take_reapable_zombies above.
    let grant_tids = {
        if let Some(sched) = SCHEDULER.lock().as_mut() {
            sched.take_pending_grant_reap()
        } else {
            alloc::vec::Vec::new()
        }
    };
    for tid in grant_tids {
        crate::task::syscall::reap_grants_for_task(tid);
        crate::hypervisor::registry::reap_vms_for_task(tid);
    }

    let hart_id = hart_local::current_hart_id();
    let switch_info = if let Some(sched) = SCHEDULER.lock().as_mut() {
        sched.pick_next(hart_id)
    } else {
        None
    };

    #[cfg(target_arch = "x86_64")]
    if switch_info.is_none() {
        unsafe {
            // No switch: re-enable interrupts before returning to the idle loop.
            core::arch::asm!("sti", options(nomem, nostack));
        }
    }

    if let Some((curr, next)) = switch_info {
        unsafe {
            let final_curr = if curr.is_null() {
                &raw mut BOOT_CONTEXT as *mut _
            } else {
                curr
            };

            let final_next = if next.is_null() {
                &raw const BOOT_CONTEXT as *const _
            } else {
                next
            };

            if !next.is_null() {
                // Set TSS.rsp0 / KERNEL_GS_BASE for the next task's syscall path.
                // On x86_64 use kernel_trap_sp (= kstack_top - TRAP_FRAME_SIZE, fixed
                // at spawn) so CPU_LOCAL.kernel_rsp never drifts to the deep
                // cooperative-switch RSP saved inside a blocked yield_cpu frame.
                let next_ref = &*next;
                #[cfg(not(target_arch = "x86_64"))]
                crate::hal::arch::set_kernel_stack(next_ref.sp as usize);
                #[cfg(target_arch = "x86_64")]
                crate::hal::arch::set_kernel_stack(next_ref.kernel_trap_sp as usize);
            }

            // switch(current, next)
            crate::hal::arch::Context::switch(final_curr, final_next);

            // Execution resumes here when this context is switched BACK to.
            // Re-enable interrupts: the cli above masked IRQs for the lock section;
            // iretq (ring-3 entry) will have re-enabled them on the other CPU path,
            // but on the resume path here we must restore IF explicitly.
            #[cfg(target_arch = "x86_64")]
            core::arch::asm!("sti", options(nomem, nostack));
        }
    }
}

pub fn spawn(name: &str, cell_id: CellId, allowed_drivers: alloc::vec::Vec<usize>) -> usize {
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        sched.spawn(name, cell_id, allowed_drivers)
    } else {
        0
    }
}

pub fn spawn_with_arg(
    name: &str,
    cell_id: CellId,
    allowed_drivers: alloc::vec::Vec<usize>,
    entry: VAddr,
    arg: usize,
) -> usize {
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        sched.spawn_thread(name, cell_id, allowed_drivers, entry, arg)
    } else {
        0
    }
}

/// Detect whether an ELF binary is PIE (ET_DYN, e_type == 3).
///
/// Reads e_type directly from the ELF header bytes (offset 16, 2 bytes LE).
/// A return value of `true` means the ELF was compiled with
/// `-C relocation-model=pic` and must be loaded at a dynamically allocated VA.
fn elf_is_pie(data: &[u8]) -> bool {
    // ELF64 header: bytes [16..18] = e_type (u16 LE).  ET_DYN == 3.
    data.len() >= 18 && u16::from_le_bytes([data[16], data[17]]) == 3
}

/// Spawn a cell from an ELF image already in memory.
///
/// Returns `(tid, load_base)` where `load_base` is:
/// - `0` for fixed-VA (non-PIE) cells — load address comes from the ELF itself.
/// - The allocated VA base for PIE cells — callers must pass this to
///   `reloc::apply_relocations` after this function returns.
pub fn spawn_from_mem(
    data: &[u8],
    name: &str,
    cell_id: CellId,
    allowed_drivers: alloc::vec::Vec<usize>,
) -> core::result::Result<(usize, usize), ViError> {
    use crate::loader::{ElfLoader, ElfParser};

    // 1. Check Magic
    if data.len() < 4 || &data[0..4] != b"\x7fELF" {
        log::error!("Spawn: Invalid ELF magic");
        return Err(ViError::InvalidInput);
    }

    // 2. Parse ELF Header
    log::info!("Spawn: parsing elf from memory ({} bytes)", data.len());

    // Ensure alignment (xmas-elf requires it)
    use alloc::vec::Vec;
    let mut _aligned_storage: Option<Vec<u8>> = None;
    let elf_data = if (data.as_ptr() as usize) % 8 != 0 {
        log::warn!("Spawn: Unaligned ELF data (0x{:X}). Copying to aligned buffer...", data.as_ptr() as usize);
        let mut v = Vec::with_capacity(data.len());
        v.extend_from_slice(data);
        _aligned_storage = Some(v);
        _aligned_storage.as_ref().unwrap()
    } else {
        data
    };

    let loader = ElfLoader;
    let header = loader.parse_header(elf_data)?;

    // 3. Determine load base.
    //    PIE cells (ET_DYN) get a fresh VA slot; fixed-VA cells use p_vaddr.
    let load_base: usize = if elf_is_pie(elf_data) {
        crate::loader::va_alloc::alloc_cell_va().ok_or_else(|| {
            log::error!("Spawn: cell VA space exhausted");
            ViError::OutOfMemory
        })?
    } else {
        0
    };

    // 4. Load Segments — capture the mapped (vaddr, frame) pairs so the cell's
    // segment frames are reclaimed when it dies (see stack::CellSegments).
    let seg_pages = {
        let mut frame_guard = crate::memory::frame::FRAME_ALLOCATOR.lock();
        let frame_allocator = frame_guard.as_mut().ok_or(ViError::OutOfMemory)?;
        loader.load_segments(elf_data, frame_allocator, load_base)
            .map_err(|e| {
                // If segment loading fails after allocating a PIE VA slot,
                // return the slot so it can be reused.
                if load_base != 0 {
                    crate::loader::va_alloc::free_cell_va(load_base);
                }
                e
            })?
    };

    // 5. Pre-allocate all per-task resources BEFORE touching the scheduler.
    //    Rust Drop ensures cleanup on any error path — no manual free needed here.
    //    load_base is transferred into CellSegments (pie_va_base field), so after
    //    this point the VA slot is freed by segments.drop(), not manually.
    let entry_va       = header.entry.wrapping_add(load_base);
    let segments       = crate::task::stack::CellSegments::new(seg_pages, load_base);
    let kstack         = crate::task::stack::Stack::new_kernel(STACK_PAGES)
        .map_err(|_| ViError::OutOfMemory)?;
    let ustack         = crate::task::stack::Stack::new_user(STACK_PAGES)
        .map_err(|_| ViError::OutOfMemory)?;
    let kstack_top     = kstack.top;
    let user_stack_top = ustack.top;

    // 6. Spawn Task — creates scheduler entry.  Resources above drop on failure.
    let tid = spawn(name, cell_id, allowed_drivers);
    if tid == 0 {
        // segments/kstack/ustack all drop here → frames, VA slot, stacks freed ✅
        return Err(ViError::Unknown);
    }

    // 7. Wire pre-allocated resources into the task under the scheduler lock.
    //    Option::take() transfers ownership; untaken Somes drop at end of scope.
    let mut segments_o = Some(segments);
    let mut kstack_o   = Some(kstack);
    let mut ustack_o   = Some(ustack);
    let mut setup_ok   = false;
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        if let Some(task) = sched.tasks.get_mut(&tid) {
            log::info!("Spawn: Setting up context for Task {}...", tid);
            task.segment_mem  = segments_o.take();
            task.kernel_stack = kstack_o.take();
            task.user_stack   = ustack_o.take();
            task.trap_frame.sepc = entry_va as _;
            task.trap_frame.sstatus = 0x20; // placeholder; overwritten per-arch below

            // Setup TrapFrame on KERNEL Stack
            let tf_ptr = kstack_top - TRAP_FRAME_SIZE;

            // Set User SP in TrapFrame
            task.trap_frame.regs[2] = user_stack_top as _;

            // CRITICAL: Set User Mode Status in TrapFrame!
            #[cfg(not(target_arch = "x86_64"))]
            { task.trap_frame.sstatus = 0x6020; }
            #[cfg(target_arch = "x86_64")]
            { task.trap_frame.sstatus = 0x202; } // RFLAGS: IF=1, reserved=1

            // Copy TrapFrame to Kernel Stack
            unsafe {
                let tf_dest = &mut *(tf_ptr as *mut crate::hal::arch::ViTrapFrame);
                *tf_dest = task.trap_frame;
            }

            // Point Context to Kernel Stack (sp field exists on all Context types)
            task.context.sp = tf_ptr as _;
            #[cfg(target_arch = "riscv64")]
            { task.context.ra = __trap_exit as *const () as usize;
              task.context.sstatus = 0x42120; } // SUM=1, FS=1, SPP=1, SPIE=1
            #[cfg(target_arch = "riscv32")]
            { task.context.ra = __trap_exit as *const () as u32;
              task.context.sstatus = 0x120_u32; } // SPP=1, SPIE=1
            #[cfg(target_arch = "aarch64")]
            {
                task.context.x30   = __trap_exit as *const () as u64;
                // SP_EL0 is banked and not auto-saved by hardware on exception entry.
                // __switch saves/restores it explicitly; seed it here so the first
                // context switch loads the correct user SP before __trap_exit also sets it.
                task.context.sp_el0 = user_stack_top as u64;
            }
            #[cfg(target_arch = "x86_64")]
            { task.context.rip = __trap_exit as *const () as u64;
              // kernel_trap_sp = fixed syscall-entry RSP; never changes after spawn.
              // yield_cpu uses this (not context.sp) for set_kernel_stack so that
              // CPU_LOCAL.kernel_rsp stays at the top of a fresh syscall frame even
              // after the task has blocked and context.sp has moved deeper.
              task.context.kernel_trap_sp = tf_ptr as u64; }

            info!(
                "Spawned ELF task '{}' (ID {}) from memory at entry 0x{:X} (load_base=0x{:X})",
                name, tid, entry_va, load_base
            );
            setup_ok = true;
        }
    }

    if !setup_ok {
        // Scheduler or task entry not found (shouldn't happen after a successful spawn,
        // but be safe).  Untaken options drop here → frames/VA/stacks freed. ✅
        // Kill the orphaned task so it never runs without a context.
        if let Some(sched) = SCHEDULER.lock().as_mut() {
            sched.exit_task(tid, 0xff);
        }
        return Err(ViError::Unknown);
    }

    Ok((tid, load_base))
}

pub fn spawn_from_file(path: &str) -> core::result::Result<usize, ViError> {
    // 1. Request file from VFS (Cell 3)
    let path_bytes = path.as_bytes();
    if path_bytes.len() > 250 {
        return Err(ViError::InvalidInput);
    }

    let mut req = [0u8; 256];
    req[0] = 1; // OpCode: GetFile
    req[1] = path_bytes.len() as u8;
    req[2..2+path_bytes.len()].copy_from_slice(path_bytes);

    // Caller ID? We are in kernel context.
    // We impersonate the current task? Or use Kernel ID (0)?
    // Protocol expects Sender ID.
    // If we use `ipc_send` directly, we can specify caller.
    // VFS replies to Sender.
    // If we say Sender is CurrentTask, VFS replies to CurrentTask.
    // CurrentTask needs to be in Recv state?
    // BUT we are in a Syscall Handler! CurrentTask is Running.
    // We cannot block in Syscall Handler waiting for IPC easily unless we yield/sleep.
    // BUT syscalls must be atomic-ish or handle blocking.
    // If we block, we set state to Waiting/Recv?
    
    // Simpler approach: Use "Synchronous" IPC via busy-wait or special kernel privilege?
    // Or just spawn from memory in `init` and avoid this complexity in kernel.
    // But `shell` needs it.
    
    // Let's rely on standard IPC mechanisms.
    // We need to send, then wait for reply.
    // This is hard inside a syscall handler without async/await or state machine.
    
    // Hack: Busy loop/Yield loop waiting for VFS reply?
    // Since VFS is on another core or time-sliced, we must yield.
    
    // For now, let's implement a blocking IPC exchange using polling?
    // We can't easily pollute the task state machine.
    
    log::error!("spawn_from_file: Kernel-side VFS request not fully implemented due to blocking complexity.");
    Err(ViError::NotSupported)
}

pub fn current_task_id() -> usize {
    hart_local::ready::current_task_id_for(hart_local::current_hart_id())
}

pub fn has_ready_tasks() -> bool {
    hart_local::ready::total_ready_count() > 0
}

// Helper to resolve path relative to CWD
fn resolve_path(cwd: &str, path: &str) -> alloc::string::String {
    if path.starts_with('/') {
        alloc::string::String::from(path)
    } else {
        // Simple path joining
        let mut p = alloc::string::String::from(cwd);
        if !p.ends_with('/') {
            p.push('/');
        }
        p.push_str(path);
        // TODO: Handle ".." and "." canonicalization
        p
    }
}

// --- File System Syscall Handlers ---

#[allow(clippy::result_unit_err)]
pub fn file_open(path: &str) -> core::result::Result<usize, ()> {
    // 1. Resolve path
    let full_path = if let Some(sched) = SCHEDULER.lock().as_mut() {
        if let Some(task) = sched.current_task_mut() {
            resolve_path(&task.cwd, path)
        } else {
            // Should not happen
            String::from(path)
        }
    } else {
        return Err(());
    };

    // 2. Open file via VIFS
    // We loop to acquire FS lock to avoid deadlock with scheduler lock?
    // No, here we don't hold scheduler lock while calling FS.

    // Check if VIFS1 is initialized
    use crate::fs::VIFS1;
    let file = {
        let mut fs_lock = VIFS1.lock();
        if let Some(fs) = fs_lock.as_mut() {
            fs.open(&full_path, api::fs::OpenMode::Read)
                .map_err(|_| ())?
        } else {
            return Err(());
        }
    };

    // 3. Store in Task
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        if let Some(task) = sched.current_task_mut() {
            let fd = task.open_files.keys().max().map(|k| k + 1).unwrap_or(3); // Start FD at 3 (0,1,2 reserved)
            task.open_files
                .insert(fd, crate::task::tcb::FileHandle::new(file));
            return Ok(fd);
        }
    }

    // Task terminated concurrently?
    Err(())
}

pub fn file_read(fd: usize, buf: &mut [u8]) -> usize {
    if fd == 0 {
        // Stdin (Keyboard)
        if buf.is_empty() {
            return 0;
        }

        let mut cons = crate::task::drivers::console_drv::CONSOLE.lock();
        cons.poll();
        let b = cons.read_byte();
        if let Some(byte) = b {
            buf[0] = byte;
            return 1;
        }
        return 0;
    }

    // File Read — synchronous. The VIFS1 ramdisk is synchronous, and the async
    // path (read_async → pending_future + state=Polling) called straight back into
    // this same sync `read()` anyway. But it returned a dummy 0 to the caller while
    // the future was never driven to completion, so a blocking reader (e.g. DOOM's
    // WAD load) received 0 bytes and an uninitialized buffer ("doesn't have IWAD").
    // Read directly under the SCHEDULER lock and return the real byte count.
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        if let Some(task) = sched.current_task_mut() {
            if let Some(handle) = task.open_files.get_mut(&fd) {
                return handle.read(buf).unwrap_or(0);
            }
        }
    }
    0
}

pub fn file_write(fd: usize, buf: &[u8]) -> usize {
    if fd == 1 || fd == 2 {
        // Stdout/Stderr
        if let Ok(s) = core::str::from_utf8(buf) {
            crate::task::print_user_log(s);
            return buf.len();
        }
        return 0;
    }
    0
}

pub fn file_close(fd: usize) {
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        if let Some(task) = sched.current_task_mut() {
            task.open_files.remove(&fd);
        }
    }
}

pub fn file_readdir(fd: usize, buf: &mut [u8]) -> core::result::Result<usize, ()> {
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        if let Some(task) = sched.current_task_mut() {
            if let Some(handle) = task.open_files.get_mut(&fd) {
                match handle.read_dir() {
                    Ok(Some(entry)) => {
                        // Serialize DirEntry to buf
                        // Entry size is 64 + 1 + 8 + padding = 73+ ? sizeof(DirEntry)
                        // types::DirEntry is repr(C).
                        // We copy bytes directly.
                        let ptr = &entry as *const _ as *const u8;
                        let size = core::mem::size_of::<types::DirEntry>();
                        if buf.len() < size {
                            return Err(());
                        }

                        unsafe {
                            core::ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), size);
                        }
                        return Ok(size);
                    }
                    Ok(None) => return Ok(0), // EOF
                    Err(_) => return Err(()),
                }
            }
        }
    }
    Err(())
}

pub fn file_fstat(_fd: usize, _stat_ptr: usize) -> core::result::Result<usize, ()> {
    Err(())
}

pub fn file_chdir(_path: &str) -> core::result::Result<usize, ()> {
    // TODO: Implement chdir
    Ok(0)
}

pub fn file_seek(fd: usize, offset: isize, whence: usize) -> core::result::Result<usize, ()> {
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        if let Some(task) = sched.current_task_mut() {
            if let Some(handle) = task.open_files.get_mut(&fd) {
                let pos = match whence {
                    0 => api::fs::SeekFrom::Start(offset as u64),
                    1 => api::fs::SeekFrom::Current(offset as i64),
                    2 => api::fs::SeekFrom::End(offset as i64),
                    _ => return Err(()), // Invalid whence
                };
                
                if let Ok(new_pos) = handle.seek(pos) {
                    return Ok(new_pos as usize);
                }
            }
        }
    }
    Err(())
}

pub fn file_remove(path: &str) -> core::result::Result<usize, ()> {
    // 1. Resolve path
    let full_path = if let Some(sched) = SCHEDULER.lock().as_mut() {
        if let Some(task) = sched.current_task_mut() {
            resolve_path(&task.cwd, path)
        } else {
            String::from(path)
        }
    } else {
        return Err(());
    };

    use crate::fs::VIFS1;
    let mut fs_lock = VIFS1.lock();
    if let Some(fs) = fs_lock.as_mut() {
        if fs.remove(&full_path).is_ok() {
            return Ok(0);
        }
    }
    Err(())
}

pub fn file_rename(_old: &str, _new: &str) -> core::result::Result<usize, ()> {
    // TODO: Implement rename in ViFileSystem trait first
    Err(()) 
}

pub fn file_getcwd(_buf: &mut [u8]) -> core::result::Result<usize, ()> {
    Err(())
}
use crate::task::tcb::LeaseAttributes;
use log::warn;

pub fn ipc_lend(
    _lender_id: usize,
    target_id: usize,
    ptr: VAddr,
    len: usize,
    flags: u32,
) -> core::result::Result<usize, ()> {
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        if let Some(target_task) = sched.tasks.get_mut(&target_id) {
            let lease_id = target_task.add_lease(ptr, len, LeaseAttributes(flags));
            return Ok(lease_id);
        }
    }
    Err(())
}

pub fn ipc_send(
    caller_id: usize,
    target_id: usize,
    msg_ptr: VAddr,
    msg_len: usize,
) -> core::result::Result<usize, ()> {
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        if !sched.tasks.contains_key(&target_id) {
            // Expected: caller sends to a cell that already exited (e.g. input service
            // dispatching to a focused cell that called sys_exit).  Not a system error.
            log::debug!("IPC: Target Task {} not found (cell exited)", target_id);
            return Err(());
        }

        let target_ready = if let Some(target) = sched.tasks.get(&target_id) {
            match target.state {
                TaskState::Recv {
                    mask: _,
                    buf_ptr,
                    buf_len,
                    ..
                } => Some((buf_ptr, buf_len)),
                _ => None,
            }
        } else {
            None
        };

        if let Some((dest_ptr, dest_len)) = target_ready {
            let app_src = msg_ptr as *const u8;
            let app_dst = dest_ptr as *mut u8;
            let copy_len = core::cmp::min(msg_len, dest_len);
            unsafe {
                core::ptr::copy_nonoverlapping(app_src, app_dst, copy_len);
            }

            if let Some(target) = sched.tasks.get_mut(&target_id) {
                target.state = TaskState::Ready;
                target.current_caller = Some(caller_id);
            }
            let prio = sched.push_ready(target_id);
            sched.pend_preempt_if_needed(prio);

            // Message was immediately delivered — caller stays runnable.
            // Do NOT set Sending here: the Send handler returns Ok(0) without
            // calling yield_cpu(), so the task goes back to userspace still in
            // Running state.  Setting Sending here causes the scheduler's
            // pick_next to skip the task on the next timer tick (it only
            // requeues Running tasks), permanently deadlocking the caller.
            return Ok(0);
        } else {
            if let Some(caller) = sched.tasks.get_mut(&caller_id) {
                caller.state = TaskState::Sending {
                    target: target_id,
                    msg_ptr,
                    msg_len,
                };
            }
            return Ok(1);
        }
    }
    Err(())
}

pub fn ipc_recv(
    caller_id: usize,
    mask: usize,
    buf_ptr: VAddr,
    buf_len: usize,
) -> core::result::Result<usize, ()> {
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        let mut found_sender = None;
        for (tid, task) in sched.tasks.iter() {
            if let TaskState::Sending {
                target,
                msg_ptr,
                msg_len,
            } = task.state
            {
                if target == caller_id {
                    found_sender = Some((*tid, msg_ptr, msg_len));
                    break;
                }
            }
        }

        if let Some((sender_id, src_ptr, src_len)) = found_sender {
            let app_src = src_ptr as *const u8;
            let app_dst = buf_ptr as *mut u8;
            let copy_len = core::cmp::min(src_len, buf_len);
            unsafe {
                core::ptr::copy_nonoverlapping(app_src, app_dst, copy_len);
            }

            // Wake the sender so it can proceed (call sys_recv for the reply,
            // or continue execution if it didn't need a reply).
            // Without this, the sender stays blocked in Sending state forever
            // after we copy its message — the IPC protocol has no other
            // mechanism to unblock it unless ipc_reply is used.
            if let Some(sender_task) = sched.tasks.get_mut(&sender_id) {
                sender_task.state = TaskState::Ready;
                sched.push_ready(sender_id);
            }

            if let Some(caller) = sched.tasks.get_mut(&caller_id) {
                caller.current_caller = Some(sender_id);
            }
            return Ok(sender_id);
        } else {
            if let Some(caller) = sched.tasks.get_mut(&caller_id) {
                caller.state = TaskState::Recv {
                    mask,
                    buf_ptr,
                    buf_len,
                    deadline: None, // no timeout by default; RecvTimeout sets this
                };
            }
            return Ok(0);
        }
    }
    Err(())
}

/// Kernel-internal IPC send helper for the hotswap orchestrator.
pub fn send_to(target: usize, msg: &[u8]) -> types::ViResult<()> {
    let caller = current_task_id();
    ipc_send(caller, target, msg.as_ptr() as usize, msg.len())
        .map(|_| ())
        .map_err(|_| types::ViError::IO)
}

/// Kernel-internal IPC recv helper for the hotswap orchestrator.
pub fn recv_from(_source: usize, buf: &mut [u8]) -> types::ViResult<usize> {
    let caller = current_task_id();
    // mask = 0 → accept from any sender (hotswap waits for the target cell's reply).
    ipc_recv(caller, 0, buf.as_mut_ptr() as usize, buf.len())
        .map_err(|_| types::ViError::IO)
}

pub fn ipc_try_recv(
    caller_id: usize,
    _mask: usize,
    buf_ptr: VAddr,
    buf_len: usize,
) -> core::result::Result<usize, ()> {
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        let mut found_sender = None;
        for (tid, task) in sched.tasks.iter() {
            if let TaskState::Sending {
                target,
                msg_ptr,
                msg_len,
            } = task.state
            {
                if target == caller_id {
                    found_sender = Some((*tid, msg_ptr, msg_len));
                    break;
                }
            }
        }

        if let Some((sender_id, src_ptr, src_len)) = found_sender {
            let app_src = src_ptr as *const u8;
            let app_dst = buf_ptr as *mut u8;
            let copy_len = core::cmp::min(src_len, buf_len);
            unsafe {
                core::ptr::copy_nonoverlapping(app_src, app_dst, copy_len);
            }

            // Wake the sender so it can call sys_recv to receive our reply.
            // Without this, the sender stays in Sending state — when we call
            // sys_send(sender, reply) the sender is not in Recv state and the
            // reply send blocks, creating a deadlock.
            if let Some(sender_task) = sched.tasks.get_mut(&sender_id) {
                sender_task.state = TaskState::Ready;
                sched.push_ready(sender_id);
            }

            if let Some(caller) = sched.tasks.get_mut(&caller_id) {
                caller.current_caller = Some(sender_id);
            }
            return Ok(sender_id);
        } else {
            return Ok(0);
        }
    }
    Err(())
}

pub fn ipc_reply(caller_id: usize, result: usize) -> core::result::Result<usize, ()> {
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        let target_id = sched.tasks.get(&caller_id).and_then(|t| t.current_caller);
        if let Some(tid) = target_id {
            if let Some(t) = sched.tasks.get_mut(&tid) {
                t.state = TaskState::Ready;
                t.reply_value = Some(result);
            }
            let prio = sched.push_ready(tid);
            sched.pend_preempt_if_needed(prio);
            if let Some(task) = sched.tasks.get_mut(&caller_id) {
                task.current_caller = None;
            }
            return Ok(0);
        }
    }
    Err(())
}

pub fn ipc_borrow_read(
    caller_id: usize,
    lease_id: usize,
    offset: usize,
    dst_ptr: VAddr,
    len: usize,
) -> core::result::Result<usize, ()> {
    if let Some(sched) = SCHEDULER.lock().as_ref() {
        if let Some(task) = sched.tasks.get(&caller_id) {
            if let Some(lease) = task.get_lease(lease_id) {
                if !lease.attributes.contains(LeaseAttributes::READ) {
                    return Err(());
                }
                // Use checked_add to prevent `offset + len` wraparound which
                // would otherwise let a caller construct an arbitrary R/W
                // primitive into the lease's surrounding memory.
                let end = offset.checked_add(len).ok_or(())?;
                if end > lease.len {
                    return Err(());
                }
                let src = lease.ptr.checked_add(offset).ok_or(())?;
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        src as *const u8,
                        dst_ptr as *mut u8,
                        len,
                    );
                }
                return Ok(len);
            }
        }
    }
    Err(())
}

pub fn ipc_borrow_write(
    caller_id: usize,
    lease_id: usize,
    offset: usize,
    src_ptr: VAddr,
    len: usize,
) -> core::result::Result<usize, ()> {
    if let Some(sched) = SCHEDULER.lock().as_ref() {
        if let Some(task) = sched.tasks.get(&caller_id) {
            if let Some(lease) = task.get_lease(lease_id) {
                if !lease.attributes.contains(LeaseAttributes::WRITE) {
                    return Err(());
                }
                let end = offset.checked_add(len).ok_or(())?;
                if end > lease.len {
                    return Err(());
                }
                let dst = lease.ptr.checked_add(offset).ok_or(())?;
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        src_ptr as *const u8,
                        dst as *mut u8,
                        len,
                    );
                }
                return Ok(len);
            }
        }
    }
    Err(())
}

pub fn ipc_grant(
    caller_id: usize,
    target_id: usize,
    ptr: VAddr,
    len: usize,
    flags: u32,
) -> core::result::Result<usize, ()> {
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        if let Some(target) = sched.tasks.get_mut(&target_id) {
            let gid = target.add_grant(ptr, len, flags, caller_id);
            return Ok(gid);
        }
    }
    Err(())
}

pub fn ipc_map(caller_id: usize, grant_id: usize) -> core::result::Result<usize, ()> {
    if let Some(sched) = SCHEDULER.lock().as_ref() {
        if let Some(task) = sched.tasks.get(&caller_id) {
            if let Some(grant) = task.get_grant(grant_id) {
                return Ok(grant.ptr);
            }
        }
    }
    Err(())
}

/// Get scheduler statistics
pub fn scheduler_stats() -> (usize, usize) {
    let task_count = if let Some(sched) = SCHEDULER.lock().as_ref() { sched.tasks.len() } else { 0 };
    (task_count, hart_local::ready::total_ready_count())
}

pub fn futex_wait(caller_id: usize, addr: VAddr, val: u32) -> core::result::Result<usize, ()> {
    // Check condition
    unsafe {
        let current_val = *(addr as *const u32);
        if current_val != val {
            return Err(()); // EAGAIN
        }
    }

    if let Some(sched) = SCHEDULER.lock().as_mut() {
        if let Some(task) = sched.tasks.get_mut(&caller_id) {
            task.state = TaskState::FutexWait { addr };
            return Ok(0);
        }
    }
    Err(())
}

pub fn futex_wake(_caller_id: usize, addr: VAddr, count: usize) -> core::result::Result<usize, ()> {
    let mut woken = 0;
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        let mut to_wake = alloc::vec::Vec::new();

        // Scan for waiting tasks
        for (tid, task) in sched.tasks.iter() {
            // Skip self? Futex wake usually doesn't wake self (self is running).
            if let TaskState::FutexWait { addr: wa_addr } = task.state {
                if wa_addr == addr {
                    to_wake.push(*tid);
                    if to_wake.len() >= count {
                        break;
                    }
                }
            }
        }

        woken = to_wake.len();

        // Wake them up
        for tid in to_wake {
            if let Some(task) = sched.tasks.get_mut(&tid) {
                task.state = TaskState::Ready;
                sched.push_ready(tid);
            }
        }
    }
    Ok(woken)
}

/// Tracks whether the console cursor is at the start of a line, so the "USER: "
/// prefix is emitted ONCE per line rather than once per `sys_log` call. Without
/// this, `print()` (no trailing newline — used for the shell prompt and per-key
/// echo) would force a prefix+newline on every byte, so typing "help" rendered as
/// four "USER: h/e/l/p" lines instead of an inline "USER: help".
static USER_LOG_AT_LINE_START: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(true);

pub fn print_user_log(msg: &str) {
    use core::sync::atomic::Ordering;
    // USER stdout must ALWAYS reach the console, independent of the kernel log
    // level — it is cell application output, not kernel debug chatter. Writing
    // straight to the UART (not via info!) lets us quiet boot-time kernel Info
    // spam without also silencing the shell prompt / cell output.
    //
    // Emit the raw bytes verbatim (no trim, no synthesised newline) so the
    // distinction between print() and println() at the ostd layer is preserved:
    // print() concatenates inline; println() ends the line. The "USER: " prefix
    // is injected only at each line start, keeping log scrapers/tests matching
    // while making interactive echo behave like a real terminal.
    let mut rest = msg;
    while !rest.is_empty() {
        if USER_LOG_AT_LINE_START.load(Ordering::Relaxed) {
            crate::task::drivers::uart::write_console("USER: ");
            USER_LOG_AT_LINE_START.store(false, Ordering::Relaxed);
        }
        match rest.find('\n') {
            Some(i) => {
                crate::task::drivers::uart::write_console(&rest[..=i]);
                USER_LOG_AT_LINE_START.store(true, Ordering::Relaxed);
                rest = &rest[i + 1..];
            }
            None => {
                crate::task::drivers::uart::write_console(rest);
                rest = "";
            }
        }
    }
}

/// Spawns a synthetic task for testing User Mode without filesystem
pub fn spawn_synthetic(
    name: &str,
    cell_id: CellId,
    entry: VAddr,
) -> core::result::Result<usize, ViError> {
    // use hal::paging::PAGE_SIZE;

    // 1. Spawn Task (Allocates stack, etc.)
    let tid = spawn(name, cell_id, alloc::vec::Vec::new());
    if tid == 0 {
        return Err(ViError::Unknown);
    }

    // 2. Map Code Page at 'entry'
    {
        let mut frame_guard = crate::memory::frame::FRAME_ALLOCATOR.lock();
        let allocator = frame_guard.as_mut().ok_or(ViError::OutOfMemory)?;
        let frame = allocator.allocate_frame().ok_or(ViError::OutOfMemory)?;

        // Write code to frame (Physical access)
        // Code: ecall (0x00000073) + loop (j .)
        // Write code to frame (Physical access)
        unsafe {
            let base = frame as *mut u8;

            // 1. lui a0, 0x1      => a0 = 0x1000 (Page Base)
            *(base as *mut u32) = 0x00001537;

            // 2. addi a0, a0, 32  => a0 = 0x1020 (String Address)
            *(base.add(4) as *mut u32) = 0x02050513;

            // 3. li a1, 21        => a1 = 21 (Length)
            *(base.add(8) as *mut u32) = 0x01500593;

            // 4. li a7, 11        => a7 = 11 (Syscall::Log)
            *(base.add(12) as *mut u32) = 0x00b00893;

            // 5. ecall
            *(base.add(16) as *mut u32) = 0x00000073;

            // 6. j .              => Loop forever
            *(base.add(20) as *mut u32) = 0x0000006F;

            // Data: "Hello from Userspace!" at offset 32
            let msg = b"Hello from Userspace!";
            core::ptr::copy_nonoverlapping(msg.as_ptr(), base.add(32), msg.len());
        }

        // Permissions: VALID | READ | EXECUTE | USER
        // Note: Generic PageFlags bits might not match RISC-V perfectly if not verified,
        // but we verified they DO match in hal implementation.
        // Or we use hal::PageFlags directly.
        use crate::memory::paging::Flags;
        // 1=V, 2=R, 8=X, 16=U ? No.
        // Check lib.rs: V=1, R=2, W=4, X=8, U=16
        // We want V, R, X, U. 1|2|8|16 = 27 (0x1B).

        let flags = Flags::from_bits(
            Flags::VALID
                | Flags::READ
                | Flags::WRITE
                | Flags::EXECUTE
                | Flags::USER
                | Flags::ACCESSED
                | Flags::DIRTY,
        );

        crate::memory::paging::map_page(allocator, entry, frame, flags)
            .map_err(|_| ViError::OutOfMemory)?;
    }

    // 3. Update Task Context (Copied from spawn_from_file)
    if let Some(sched) = SCHEDULER.lock().as_mut() {
        if let Some(task) = sched.tasks.get_mut(&tid) {
            task.trap_frame.sepc = entry as _;
            task.trap_frame.sstatus = 0x20; // User Mode (SPIE=1, SPP=0)

            // Allocate Kernel Stack
            let kstack = match crate::task::stack::Stack::new_kernel(STACK_PAGES) {
                Ok(s) => s,
                Err(_) => return Err(ViError::OutOfMemory),
            };

            // Allocate User Stack
            let ustack = match crate::task::stack::Stack::new_user(STACK_PAGES) {
                Ok(s) => s,
                Err(_) => return Err(ViError::OutOfMemory),
            };

            // Use Stacks
            let kstack_top = kstack.top;
            let user_stack_top = ustack.top;

            task.kernel_stack = Some(kstack);
            task.user_stack = Some(ustack);

            let tf_ptr = kstack_top - TRAP_FRAME_SIZE;
            task.trap_frame.regs[2] = user_stack_top as _; // User SP

            unsafe {
                let tf_dest = &mut *(tf_ptr as *mut crate::hal::arch::ViTrapFrame);
                *tf_dest = task.trap_frame;
            }

            task.context.sp = tf_ptr as _;
            #[cfg(target_arch = "riscv64")]
            { task.context.ra = __trap_exit as *const () as usize;
              task.context.sstatus = 0x40120; } // SUM=1
            #[cfg(target_arch = "riscv32")]
            { task.context.ra = __trap_exit as *const () as u32;
              task.context.sstatus = 0x120_u32; } // SPP=1, SPIE=1
            #[cfg(target_arch = "aarch64")]
            {
                task.context.x30   = __trap_exit as *const () as u64;
                task.context.sp_el0 = user_stack_top as u64;
            }
            #[cfg(target_arch = "x86_64")]
            { task.context.rip = __trap_exit as *const () as u64; }

            info!(
                "Spawned Synthetic task '{}' (ID {}) at entry 0x{:X}",
                name, tid, entry
            );
        }
    }

    Ok(tid)
}
