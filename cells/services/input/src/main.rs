#![no_std]
#![no_main]
// Note: #[no_mangle] on main() is required by the ViOS ELF loader and triggers
// unsafe_attr, so we cannot use #![forbid(unsafe_code)] globally here.
// All business logic in the submodules is unsafe-free.

//! Input Service Cell.
//!
//! Receives raw EV_KEY events from the kernel VirtIO input driver via IPC,
//! translates scancodes to `InputEvent`s using the US QWERTY layout, and
//! dispatches them to the currently focused cell.
//!
//! ## IPC protocol (inbound from kernel)
//! ```text
//! byte[0]   = event type: 0=EV_KEY
//! byte[1..5]= scancode (u32 LE)
//! byte[5..9]= value    (u32 LE: 0=release, 1=press, 2=repeat)
//! ```
//!
//! ## IPC protocol (outbound to focused cell)
//! See `dispatcher::Dispatcher::dispatch` and `api::input::encode_event`.

extern crate alloc;

mod dispatcher;
mod layout_us_qwerty;
mod modifier_state;

use api::input::{InputEvent, KeyEvent};
use dispatcher::Dispatcher;
use layout_us_qwerty::{translate, key_state_from_evdev};
use modifier_state::ModifierState;
use ostd::io::println;
use ostd::syscall::{sys_recv, sys_get_time, SyscallResult};

/// Raw event type discriminant for keyboard events.
const EV_KEY: u8 = 0;
/// Opcode in inbound IPC messages from the kernel's VirtIO input driver.
const OP_SET_FOCUS: u8 = 0x20; // kernel or compositor sends this to change focus

/// Input Cell entry point.
///
/// Runs an infinite receive loop, translating and dispatching every raw event.
#[no_mangle]
pub fn main() {
    println("[input] Input Service v0.2: US QWERTY + focus routing");

    let mut modifiers = ModifierState::new();
    let mut dispatcher = Dispatcher::new();
    let mut buf = [0u8; 64];

    loop {
        match sys_recv(0, &mut buf) {
            SyscallResult::Ok(sender) if sender > 0 => {
                handle_message(&buf, sender, &mut modifiers, &mut dispatcher);
            }
            _ => {
                ostd::task::yield_now();
            }
        }
    }
}

/// Process one incoming IPC message.
fn handle_message(
    buf: &[u8; 64],
    _sender: usize,
    modifiers: &mut ModifierState,
    dispatcher: &mut Dispatcher,
) {
    match buf[0] {
        EV_KEY => {
            if buf.len() < 9 { return; }
            let scancode = u32::from_le_bytes([buf[1], buf[2], buf[3], buf[4]]);
            let value    = u32::from_le_bytes([buf[5], buf[6], buf[7], buf[8]]);
            let state    = key_state_from_evdev(value);

            // Update modifier state; if it was a modifier-only key, skip dispatch.
            if modifiers.update(scancode, state) {
                return;
            }

            let (keysym, character) = translate(scancode, modifiers.snapshot());
            let ev = InputEvent::Key(KeyEvent {
                timestamp_ticks: sys_get_time(),
                scancode,
                keysym,
                character,
                modifiers: modifiers.snapshot(),
                state,
                _pad: [0; 2],
            });
            dispatcher.dispatch(&ev);
        }
        OP_SET_FOCUS => {
            // Payload: bytes [1..9] = new focus endpoint (u64 LE).
            if buf.len() < 9 { return; }
            let endpoint = u64::from_le_bytes([
                buf[1], buf[2], buf[3], buf[4],
                buf[5], buf[6], buf[7], buf[8],
            ]) as usize;
            modifiers.reset_transient(); // clear stuck modifiers on focus change
            dispatcher.set_focus(endpoint);
        }
        _ => {
            // Unknown opcode — ignore.
        }
    }
}
