//! Trap frame structures and S-mode trap handling for ViCell.
//! Uses Vi prefix per project conventions (Luật 6).
//! TrapFrame uses borrowing (&mut) per Luật 8.

/// Trap frame saved on stack during exception/interrupt.
/// Must match the layout in trap.S exactly!
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct ViTrapFrame {
    pub regs: [usize; 32], // x0-x31 (x0 always 0 but slot exists)
    pub sstatus: usize,
    pub sepc: usize,
    pub stval: usize,
    pub scause: usize,
}

impl ViTrapFrame {
    pub fn new() -> Self {
        Self::default()
    }
}

// External assembly functions
extern "C" {
    fn __trap_entry();
    pub fn vi_set_sscratch(kernel_stack_top: usize);
}

/// Initialize trap handling by setting stvec
pub fn init() {
    unsafe {
        let trap_entry = __trap_entry as *const () as usize;
        // Set stvec to direct mode (all traps go to __trap_entry)
        core::arch::asm!("csrw stvec, {}", in(reg) trap_entry);
        // Initialize sscratch to 0 (indicates S-mode context)
        core::arch::asm!("csrw sscratch, zero");
    }
}

/// Set sscratch to kernel stack top before switching to userspace
/// Call this before context switch to user mode
pub fn set_kernel_stack(kernel_stack_top: usize) {
    unsafe {
        vi_set_sscratch(kernel_stack_top);
    }
}

pub fn enable_interrupts() {
    unsafe {
        #[cfg(target_arch = "riscv64")]
        core::arch::asm!("csrsi sstatus, 0x2"); // SIE
    }
}

/// Rust trap handler called from assembly (vi_trap_handler)
/// Uses borrowed &mut ViTrapFrame per Luật 8
/// This function handles all traps: syscalls, interrupts, exceptions
#[no_mangle]
pub extern "C" fn vi_trap_handler(frame: &mut ViTrapFrame) {
    let scause = frame.scause;
    let is_interrupt = (scause >> 63) != 0;
    let code = scause & 0x7FFF_FFFF_FFFF_FFFF;

    if is_interrupt {
        // Handle interrupts
        match code {
            1 => {
                // S-mode software interrupt — zero-latency RT preemption.
                // Cleared here before yield so it does not re-fire immediately.
                // SAFETY: csrci on sip.SSIP is permitted from S-mode (priv spec §4.1.3).
                unsafe { core::arch::asm!("csrci sip, 0x2") };
                // Reuse the timer tick path: just run the scheduler.
                unsafe { vi_timer_tick(); }
            }
            5 => {
                // S-mode timer interrupt — preemption point.
                // SAFETY: vi_timer_tick is defined in kernel::task and linked
                // via extern "Rust".  It increments the tick counter, rearmed
                // the timer, and calls yield_cpu() to preempt if needed.
                unsafe { vi_timer_tick(); }
            }
            9 => {
                // S-mode external interrupt (PLIC)
                // Claim first, dispatch handler, complete AFTER handler per PLIC spec.
                if let Some(irq) = plic_claim() {
                    if irq >= 1 && irq <= 8 {
                        // SAFETY: vi_handle_virtio_irq is defined in kernel/src/task/drivers/virtio_blk.rs
                        // and linked via extern "Rust". The irq argument is a valid PLIC claim value (1-8).
                        unsafe { vi_handle_virtio_irq(irq); }
                    } else if irq == 10 {
                        // SAFETY: vi_handle_uart_irq is defined in the kernel and linked via extern "Rust".
                        unsafe { vi_handle_uart_irq(); }
                    }
                    // PLIC complete must come AFTER the device handler has run.
                    plic_complete(irq);
                }
            }
            _ => {
                // Unknown interrupt - log but don't panic
                // log::warn!("Unknown interrupt: {}", code);
            }
        }
    } else {
        // Handle exceptions
        match code {
            8 => {
                // Environment call from U-mode (syscall)
                vi_handle_syscall(frame);
                // Advance PC past ecall instruction (4 bytes)
                frame.sepc += 4;
            }
            9 => {
                // Environment call from S-mode (should not happen normally)
                frame.sepc += 4;
            }
            2 => {
                // Illegal instruction
                panic!(
                    "ViCell: Illegal instruction at 0x{:X}, stval=0x{:X}",
                    frame.sepc, frame.stval
                );
            }
            12 => {
                // Instruction page fault
                panic!(
                    "ViCell: Instruction page fault at 0x{:X}, addr=0x{:X}",
                    frame.sepc, frame.stval
                );
            }
            13 => {
                // Load page fault
                panic!(
                    "ViCell: Load page fault at 0x{:X}, addr=0x{:X}",
                    frame.sepc, frame.stval
                );
            }
            15 => {
                // Store page fault
                panic!(
                    "ViCell: Store page fault at 0x{:X}, addr=0x{:X}",
                    frame.sepc, frame.stval
                );
            }
            _ => {
                panic!(
                    "ViCell: Unhandled exception: scause={}, sepc=0x{:X}, stval=0x{:X}",
                    code, frame.sepc, frame.stval
                );
            }
        }
    }
}

/// Claim the highest-priority pending IRQ from PLIC (S-mode context 1).
/// Returns the IRQ number, or None if no interrupt is pending.
/// The caller MUST call `plic_complete(irq)` after the device handler returns.
fn plic_claim() -> Option<u32> {
    use crate::common::plic::PLIC;
    let irq = PLIC.claim(1);
    if irq != 0 { Some(irq) } else { None }
}

/// Notify PLIC that IRQ handling is complete (S-mode context 1).
/// Must be called AFTER the device handler has finished.
fn plic_complete(irq: u32) {
    use crate::common::plic::PLIC;
    PLIC.complete(1, irq);
}

/// Handle syscall from userspace (Vi prefix per Luật 6)
fn vi_handle_syscall(frame: &mut ViTrapFrame) {
    extern "Rust" {
        fn ViCell_syscall_dispatch(frame: &mut ViTrapFrame);
    }
    unsafe {
        ViCell_syscall_dispatch(frame);
    }
}

extern "Rust" {
    fn vi_handle_virtio_irq(irq: u32);
    fn vi_handle_uart_irq();
    /// Called on every S-mode timer interrupt.  Defined in `kernel::task`.
    fn vi_timer_tick();
}
