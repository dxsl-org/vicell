use crate::sync::Spinlock;
use crate::task::drivers::virtio_hal::VirtioHal;
use alloc::collections::VecDeque;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicUsize, Ordering};
use virtio_drivers::device::input::VirtIOInput;
use virtio_drivers::transport::mmio::{MmioTransport, VirtIOHeader};
use virtio_drivers::transport::{DeviceType, Transport};

/// TID of the registered input service cell (0 = unregistered).
/// Set by the loader when `/bin/input` is spawned; cleared on its death.
pub static INPUT_CELL_ID: AtomicUsize = AtomicUsize::new(0);

/// Register the input service cell. Called by the loader after spawning `/bin/input`.
pub fn set_input_cell(tid: usize) {
    INPUT_CELL_ID.store(tid, Ordering::Release);
    log::info!("[input] registered input service TID {}", tid);
}

/// Clear the input service registration if it matches `tid` (called on cell death).
pub fn clear_input_cell_if(tid: usize) {
    INPUT_CELL_ID.compare_exchange(tid, 0, Ordering::AcqRel, Ordering::Relaxed).ok();
}

#[derive(Clone, Copy)]
pub struct KeyboardEvent {
    pub event_type: u16,
    pub code: u16,
    pub value: u32,
}

/// Cap on buffered input events. The kernel retains undelivered events (see
/// `dispatch_pending`) so none are lost while the focused app is busy rendering;
/// this bounds growth if the app drains far slower than the user types — oldest
/// events are dropped first, keeping the most recent input.
const MAX_QUEUED_EVENTS: usize = 256;

pub struct VirtIOInputDriver {
    pub input: VirtIOInput<VirtioHal, MmioTransport>,
    pub event_queue: VecDeque<KeyboardEvent>,
}

pub static KEYBOARD_DRIVER: Spinlock<Option<VirtIOInputDriver>> = Spinlock::new(None);

/// IRQ number of the probed input device (0 = not found).
/// Set during init so the interrupt handler can identify this device's IRQ.
pub static INPUT_DEVICE_IRQ: Spinlock<u32> = Spinlock::new(0);

/// Force-release this module's locks during fault teardown.
///
/// # Safety
/// Single-hart; called only from the fault/panic path with interrupts disabled.
pub unsafe fn force_unlock_locks() {
    KEYBOARD_DRIVER.force_unlock();
    INPUT_DEVICE_IRQ.force_unlock();
}

pub fn init_driver() {
    use crate::task::drivers::virtio_common::virtio_slots;
    for slot in virtio_slots() {
        let header = unsafe { NonNull::new_unchecked(slot.base as *mut VirtIOHeader) };
        match unsafe { MmioTransport::new(header) } {
            Ok(transport) => {
                if transport.device_type() == DeviceType::Input {
                    match VirtIOInput::<VirtioHal, MmioTransport>::new(transport) {
                        Ok(input) => {
                            log::info!("VirtIO Input: initialized at {:#x} irq={}", slot.base, slot.irq);
                            *INPUT_DEVICE_IRQ.lock() = slot.irq;
                            *KEYBOARD_DRIVER.lock() = Some(VirtIOInputDriver {
                                input,
                                event_queue: VecDeque::new(),
                            });
                            return;
                        }
                        Err(e) => log::warn!("VirtIO Input init failed at {:#x}: {:?}", slot.base, e),
                    }
                } else {
                    // MmioTransport::drop() resets the device (writes status=0).
                    // Forget the transport to avoid resetting a foreign slot that
                    // another driver (e.g. virtio_blk) will initialise.
                    core::mem::forget(transport);
                }
            }
            Err(_) => {}
        }
    }
}

/// Called from the trap handler when a VirtIO IRQ fires.
///
/// Returns `true` if the IRQ belonged to the input device and was acknowledged.
/// Failing to call this causes `InterruptStatus` to stay set, which makes the
/// PLIC re-fire the same IRQ immediately after `plic_complete` — an interrupt storm.
pub fn ack_irq(irq: u32) -> bool {
    let device_irq = *INPUT_DEVICE_IRQ.lock();
    if device_irq == 0 || device_irq != irq {
        return false;
    }
    if let Some(drv) = KEYBOARD_DRIVER.lock().as_mut() {
        drv.input.ack_interrupt();
    }
    true
}

pub fn poll_events() {
    if let Some(driver) = KEYBOARD_DRIVER.lock().as_mut() {
        while let Some(event) = driver.input.pop_pending_event() {
            log::debug!(
                "[virt-kbd] event type={} code={} value={}",
                event.event_type,
                event.code,
                event.value
            );
            if driver.event_queue.len() >= MAX_QUEUED_EVENTS {
                driver.event_queue.pop_front(); // drop oldest to bound growth
            }
            driver.event_queue.push_back(KeyboardEvent {
                event_type: event.event_type,
                code: event.code,
                value: event.value,
            });
        }
    }
}

/// Drain pending VirtIO input events and forward to the registered input service.
///
/// Safe to call from IRQ context: drains event_queue under the lock, releases
/// the lock, then sends each event via ipc_send (which acquires SCHEDULER.lock()
/// separately — no lock inversion with KEYBOARD_DRIVER).
///
/// Fire-and-forget: if the input service is not in Recv state, the event is
/// dropped. Acceptable because the input service is almost always blocking on recv.
pub fn dispatch_pending() {
    let input_tid = INPUT_CELL_ID.load(Ordering::Relaxed);
    if input_tid == 0 {
        return;
    }

    use crate::task::drivers::input_map::{EV_ABS, EV_KEY, EV_REL};

    // IPC is a rendezvous with no queue: ipc_send(0, …) delivers only when the
    // input service is parked in Recv, returning Ok(0); otherwise Ok(1) and the
    // event is NOT delivered (caller_id 0 has no task to park in Sending).
    //
    // We therefore peek the front event and dequeue it ONLY on confirmed delivery.
    // While the input service is busy (blocked forwarding the previous event to the
    // focused app), events stay buffered in event_queue and retry on the next tick —
    // so keystrokes are never silently lost while an app is mid-render. At most one
    // event is delivered per call: a successful send leaves the input service Ready
    // (no longer in Recv), so any further send this tick would fail anyway.
    loop {
        let front = {
            let guard = KEYBOARD_DRIVER.lock();
            guard.as_ref().and_then(|d| d.event_queue.front().copied())
        }; // lock released before ipc_send (avoids inversion with SCHEDULER)
        let Some(ev) = front else { return };

        let opcode: u8 = if ev.event_type as u8 == EV_KEY as u8 {
            0
        } else if ev.event_type as u8 == EV_REL as u8 {
            1
        } else if ev.event_type as u8 == EV_ABS as u8 {
            2
        } else {
            // Unknown event type — drop it so it can't wedge the queue head.
            let mut guard = KEYBOARD_DRIVER.lock();
            if let Some(d) = guard.as_mut() { d.event_queue.pop_front(); }
            continue;
        };
        let mut msg = [0u8; 9];
        msg[0] = opcode;
        msg[1..5].copy_from_slice(&(ev.code as u32).to_le_bytes());
        msg[5..9].copy_from_slice(&ev.value.to_le_bytes());

        // ipc_send copies msg into the target cell's Recv buffer (a U-mode stack page).
        // Called from the timer ISR where SUM (Supervisor User Memory, sstatus bit 18)
        // is NOT set; set it here so the S-mode copy_nonoverlapping to the U-mode page
        // does not fault with scause=15. Cleared immediately after via the guard's Drop.
        //
        // SAFETY: SUM=1 allows S-mode to access U-mode pages; cleared right after
        // ipc_send so no wider kernel code runs with SUM elevated.
        let delivered = {
            #[cfg(target_arch = "riscv64")]
            let _sum_guard = {
                unsafe { core::arch::asm!("csrs sstatus, {0}", in(reg) 0x40000usize); }
                struct SumGuard;
                impl Drop for SumGuard {
                    fn drop(&mut self) {
                        unsafe { core::arch::asm!("csrc sstatus, {0}", in(reg) 0x40000usize); }
                    }
                }
                SumGuard
            };
            matches!(crate::task::ipc_send(0, input_tid, msg.as_ptr() as usize, 9), Ok(0))
        };

        if delivered {
            let mut guard = KEYBOARD_DRIVER.lock();
            if let Some(d) = guard.as_mut() { d.event_queue.pop_front(); }
        }
        // Whether delivered (input now Ready) or not (input busy), stop here and
        // resume next tick — at most one event leaves the queue per dispatch.
        return;
    }
}
