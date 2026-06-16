//! GPIO IRQ dispatcher — routes PL061 edge interrupts to the MMIO owner cell.
//!
//! When a GPIO pin fires an edge interrupt, the GIC delivers SPI 7 (GIC ID 39)
//! to the kernel.  This module looks up the cell that currently owns the PL061
//! MMIO region (0x0903_0000) via the Resource Registry, then sends it a 4-byte
//! IPC notification: `[0x02, 0x00, 0x00, 0x00]` (opcode 2 = GPIO_IRQ_NOTIFY).
//!
//! The receiving cell reads GPIOMIS to determine which pins fired, calls
//! `clear_irq(mask)` to acknowledge, and re-enables the interrupt as needed.
//!
//! # Ownership model
//! No static registration is needed.  The Resource Registry already tracks which
//! cell called `RequestMmio(PL061_BASE, …)`, and automatically releases the
//! entry when the cell exits.  A new cell that re-opens GPIO after a restart
//! becomes the new owner transparently.

/// PL061 GPIO MMIO base on QEMU ARM virt — matches the allowlist in resource_registry.rs.
const PL061_BASE: usize = 0x0903_0000;

/// IPC opcode sent to the GPIO owner cell on interrupt.
/// Chosen to not collide with kernel raw event opcodes 0 (EV_KEY) / 1 (EV_REL) / 2 (EV_ABS).
const GPIO_IRQ_NOTIFY: u8 = 0xA0;

/// Send a GPIO IRQ notification to the current PL061 MMIO owner.
///
/// Called as `vi_gpio_notify_irq` (extern "Rust") from `vi_aarch64_irq_handler`
/// in `hal/arch/arm/src/aarch64/trap.rs` when GIC ID 39 (GPIO SPI 7) fires.
///
/// Safe to call from IRQ context: does not acquire SCHEDULER; `ipc_send` uses
/// its own internal lock.  Fire-and-forget: if the owner cell is not in `Recv`
/// state the event is dropped (the cell re-polls GPIOMIS on its next recv).
#[no_mangle]
pub extern "Rust" fn vi_gpio_notify_irq() {
    let Some(tid) = crate::resource_registry::lookup_mmio_owner(PL061_BASE) else {
        return; // No cell currently owns GPIO — drop the interrupt.
    };
    let msg = [GPIO_IRQ_NOTIFY, 0x00, 0x00, 0x00];
    // SAFETY: ipc_send copies 4 bytes from a stack buffer to the target cell's
    // recv page.  On AArch64 EL1 can access EL0 pages in ViCell's SAS layout
    // (no PAN on cortex-a57/a72 default).
    let _ = crate::task::ipc_send(0, tid, msg.as_ptr() as usize, 4);
}
