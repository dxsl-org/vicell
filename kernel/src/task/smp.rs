//! SMP: secondary hart startup and controlled park loop.
//!
//! Phase 01: brings each secondary hart online, installs its trap vector,
//! then parks it in WFI.  Phase 03 replaces the park loop with a per-hart
//! scheduler round.
//!
//! Invariant: hart 0 calls `start_secondaries()` only AFTER `task::init()`
//! completes — the SCHEDULER and heap are live before any secondary runs.

use core::sync::atomic::{AtomicBool, Ordering};

/// Maximum number of harts this kernel tracks.  2 covers QEMU virt `-smp 2`
/// (G2 entry target).  Constant so secondary stacks and HART_ONLINE are
/// statically sized — no heap allocation during the boot critical path.
pub const MAX_HARTS: usize = 2;

/// Set to `true` by each secondary hart once its trap vector and timer are ready.
/// Hart 0's bounded wait reads this via `Acquire` to observe all preceding stores.
pub static HART_ONLINE: [AtomicBool; MAX_HARTS] = [
    AtomicBool::new(false),
    AtomicBool::new(false),
];

/// How many 10 ms ticks hart 0 waits for each secondary to come online before
/// logging a warning and continuing single-hart.  500 ms is generous for QEMU.
const SECONDARY_BOOT_TIMEOUT_TICKS: usize = 50;

/// Called by hart 0 **after** `task::init()` to bring secondary harts online.
///
/// Each secondary is started via SBI HSM `hart_start`.  Hart 0 then spins
/// (bounded) waiting for each secondary to set `HART_ONLINE[hart_id]`.
/// If a secondary fails to start or times out, a warning is logged and the
/// system continues single-hart — graceful degradation, never a panic.
#[cfg(target_arch = "riscv64")]
pub fn start_secondaries() {
    use crate::task::stack::Stack;
    use crate::task::STACK_PAGES;
    use hal::common::sbi::{sbi_hart_get_status, sbi_hart_start};

    extern "C" {
        // Physical asm label defined in hal/arch/riscv/src/rv64/boot.rs.
        // Runs bare (SATP=0); no relocation or BSS clear.
        fn _secondary_entry();
    }

    for hart_id in 1..MAX_HARTS {
        // Allocate a dedicated kernel stack for this hart.  Leak it — it lives
        // for the entire lifetime of the hart.
        let stack = match Stack::new_kernel(STACK_PAGES) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("[smp] hart {} stack alloc failed: {:?}", hart_id, e);
                continue;
            }
        };
        let stack_top = stack.top;
        core::mem::forget(stack);

        if let Ok(state) = sbi_hart_get_status(hart_id) {
            log::info!("[smp] hart {} HSM state = {}", hart_id, state);
        }

        // SAFETY: _secondary_entry is a physical-address asm label; the kernel
        // is loaded at 0x80200000 with slide=0 so physical == virtual.
        // stack_top is the usable top of a freshly-allocated kernel stack.
        // SAFETY: casting function pointer to integer — use double-cast through
        // *const () to avoid the "direct cast of function item" lint.
        let entry_paddr = _secondary_entry as *const () as usize;
        match sbi_hart_start(hart_id, entry_paddr, stack_top) {
            Ok(()) => log::info!("[smp] hart {} start requested (entry={:#x})", hart_id, entry_paddr),
            Err(e) => {
                log::warn!("[smp] hart {} SBI hart_start failed: err={}", hart_id, e);
                continue;
            }
        }

        // Bounded spin: wait for the secondary to signal it is online.
        let deadline = crate::task::system_ticks() + SECONDARY_BOOT_TIMEOUT_TICKS;
        loop {
            if HART_ONLINE[hart_id].load(Ordering::Acquire) {
                log::info!("[smp] hart {} online, parked", hart_id);
                break;
            }
            if crate::task::system_ticks() >= deadline {
                log::warn!("[smp] hart {} did not come online in time — continuing single-hart", hart_id);
                break;
            }
            core::hint::spin_loop();
        }
    }
}

/// No-op on non-riscv64 targets.
#[cfg(not(target_arch = "riscv64"))]
pub fn start_secondaries() {}

/// Entry point for secondary harts, called from `_secondary_entry` asm.
///
/// a0 = hart_id (set by OpenSBI per SBI HSM §9.1.1).
///
/// Installs the trap vector on this hart, marks the hart online, then parks
/// in WFI.  Phase 03 replaces the park loop with the per-hart scheduler round.
#[no_mangle]
pub extern "C" fn smp_hart_entry(hart_id: usize) -> ! {
    // Install the trap vector on this hart (each hart has its own stvec CSR).
    // `hal::ARCH.init()` sets stvec + enables SSIE — safe to call from any hart.
    #[cfg(target_arch = "riscv64")]
    {
        use hal::Arch; // bring the Arch trait into scope for `.init()`
        hal::ARCH.init();
    }

    // Install per-hart local state so current_cell_id() is correct on this hart.
    // Phase 02 fills in the real implementation; the call is a no-op until then.
    #[cfg(target_arch = "riscv64")]
    crate::task::hart_local::install(hart_id);

    // Signal hart 0's bounded wait that we are ready.
    if hart_id < MAX_HARTS {
        HART_ONLINE[hart_id].store(true, Ordering::Release);
    }

    // Phase 03 replaces this with the per-hart scheduler loop.
    loop {
        // SAFETY: wfi is a privileged S-mode hint that suspends the hart until
        // the next interrupt.  No state is mutated; resuming after an IPI or
        // timer is idempotent for a parked hart.
        #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
        unsafe { core::arch::asm!("wfi", options(nomem, nostack)) };
        core::hint::spin_loop();
    }
}
