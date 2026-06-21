#![no_std]
#![no_main]
// Note: #[no_mangle] on main() is required by the ViCell ELF loader and triggers
// unsafe_attr, so we cannot use #![forbid(unsafe_code)] globally here.
// All business logic in the submodules is unsafe-free.

//! Input Service Cell.
//!
//! Receives raw EV_KEY events from the kernel VirtIO input driver via IPC,
//! translates scancodes to `InputEvent`s using the US QWERTY layout, and
//! dispatches them to the currently focused cell.
//!
//! ## IPC protocol (inbound from kernel, sender == 0)
//! ```text
//! byte[0]   = event type: 0=EV_KEY, 1=EV_REL, 2=EV_ABS
//! byte[1..5]= code  (u32 LE: scancode, REL_*, ABS_* axis)
//! byte[5..9]= value (u32 LE: key state, signed rel delta, abs coord)
//! ```
//! Sender 0 is the kernel; these raw frames bypass postcard decoding entirely.
//!
//! ## Focus IPC (inbound from compositor/shell, sender > 0)
//! Typed `InputRequest` encoded with postcard — see `api::ipc::InputRequest`.
//! Sender > 0 always routes to postcard decode; opcode collisions with kernel
//! frames are impossible by construction.
//!
//! ## IPC protocol (outbound to focused cell)
//! See `dispatcher::Dispatcher::dispatch` and `api::input::encode_event`.

extern crate alloc;

mod dispatcher;
mod layout_us_qwerty;
mod modifier_state;
mod mouse_state;

use api::input::{InputEvent, KeyEvent, KeyState, KeySym, Modifiers};
use api::ipc::{InputRequest, InputResponse, IPC_BUF_SIZE};
use dispatcher::Dispatcher;
use layout_us_qwerty::{translate, key_state_from_evdev};
use modifier_state::ModifierState;
use mouse_state::{MouseState, btn_to_mouse_button, BTN_LEFT};
use ostd::io::println;
use ostd::syscall::{sys_recv, sys_send, sys_get_time, SyscallResult};

api::declare_manifest!(block_io = false, network = false, spawn = false);
api::declare_syscalls![Send, Recv, Log, Heartbeat, GetTime];

/// Raw event type discriminant for keyboard events (kernel VirtIO push).
const EV_KEY: u8 = 0;
/// Raw event type for UART ASCII relay from the kernel console driver.
/// The code field carries the raw ASCII byte; no scancode translation needed.
const EV_ASCII: u8 = 0x04;

/// Input Cell entry point.
///
/// Runs an infinite receive loop, translating and dispatching every raw event.
#[no_mangle]
pub fn main() {
    println("[input] Input Service v0.3: US QWERTY + typed focus routing");

    let mut modifiers = ModifierState::new();
    let mut mouse = MouseState::new();
    let mut dispatcher = Dispatcher::new();
    let mut buf = [0u8; IPC_BUF_SIZE];

    loop {
        // Accept sender=0 (kernel raw push) and sender>0 (typed cell request).
        match sys_recv(0, &mut buf) {
            SyscallResult::Ok(sender) => {
                handle_message(&buf, sender, &mut modifiers, &mut mouse, &mut dispatcher);
            }
            _ => {
                ostd::task::yield_now();
            }
        }
    }
}

/// Process one incoming IPC message.
///
/// Discrimination is by `sender`, not opcode, to avoid collisions with postcard
/// discriminants: kernel pushes arrive with sender=0; typed requests sender>0.
fn handle_message(
    buf: &[u8; IPC_BUF_SIZE],
    sender: usize,
    modifiers: &mut ModifierState,
    mouse: &mut MouseState,
    dispatcher: &mut Dispatcher,
) {
    if sender == 0 {
        handle_kernel_event(buf, modifiers, mouse, dispatcher);
    } else {
        handle_typed_request(buf, sender, modifiers, dispatcher);
    }
}

/// Handle a raw VirtIO event pushed by the kernel (sender == 0).
///
/// Wire format: `[opcode:1][code:4 LE][value:4 LE]`
/// opcode 0 = EV_KEY (keyboard key or mouse button via BTN_* scancode ≥ 0x110)
/// opcode 1 = EV_REL (relative mouse: REL_X/Y/WHEEL)
/// opcode 2 = EV_ABS (absolute mouse: ABS_X/Y)
fn handle_kernel_event(
    buf: &[u8; IPC_BUF_SIZE],
    modifiers: &mut ModifierState,
    mouse: &mut MouseState,
    dispatcher: &mut Dispatcher,
) {
    if buf.len() < 9 { return; }
    let code  = u32::from_le_bytes([buf[1], buf[2], buf[3], buf[4]]);
    let value = u32::from_le_bytes([buf[5], buf[6], buf[7], buf[8]]);

    match buf[0] {
        EV_KEY => {
            let state = key_state_from_evdev(value);
            // BTN_* codes (≥ 0x110) are mouse buttons, not keyboard keys.
            if code >= BTN_LEFT {
                if let Some(button) = btn_to_mouse_button(code) {
                    dispatcher.dispatch(&InputEvent::MouseButton { button, state });
                }
                return;
            }
            if modifiers.update(code, state) { return; }
            let (keysym, character) = translate(code, modifiers.snapshot());
            dispatcher.dispatch(&InputEvent::Key(KeyEvent {
                timestamp_ticks: sys_get_time(),
                scancode: code,
                keysym,
                character,
                modifiers: modifiers.snapshot(),
                state,
                _pad: [0; 2],
            }));
        }
        1 => {
            if let Some(ev) = mouse.apply_rel(code, value) {
                dispatcher.dispatch(&ev);
            }
        }
        2 => {
            if let Some(ev) = mouse.apply_abs(code, value) {
                dispatcher.dispatch(&ev);
            }
        }
        EV_ASCII => {
            // UART byte relayed by the kernel console driver.
            // `code` carries the raw ASCII code point; skip scancode translation.
            // Map C0 control chars to semantic KeySyms so GUI apps get proper events
            // regardless of whether input originates from VirtIO or UART terminal.
            let state = if value > 0 { KeyState::Pressed } else { KeyState::Released };
            let (keysym, character) = match code {
                0x1B        => (KeySym::Escape,    0),
                0x0D | 0x0A => (KeySym::Return,    code),
                0x08 | 0x7F => (KeySym::Backspace, code),
                0x09        => (KeySym::Tab,        code),
                _           => (KeySym::Printable,  code),
            };
            dispatcher.dispatch(&InputEvent::Key(KeyEvent {
                timestamp_ticks: sys_get_time(),
                scancode: 0,
                keysym,
                character,
                modifiers: modifiers.snapshot(),
                state,
                _pad: [0; 2],
            }));
        }
        _ => {} // unknown opcode — drop silently
    }
}

/// Handle a typed `InputRequest` from a compositor or shell cell (sender > 0).
fn handle_typed_request(
    buf: &[u8; IPC_BUF_SIZE],
    sender: usize,
    modifiers: &mut ModifierState,
    dispatcher: &mut Dispatcher,
) {
    let mut resp_buf = [0u8; 64];
    match api::ipc::decode::<InputRequest>(buf) {
        Ok(InputRequest::SetFocus { cell_tid: _ }) => {
            modifiers.reset_transient();
            // Use kernel-verified sender TID instead of the cell_tid field to
            // prevent a cell from redirecting focus to an arbitrary TID.
            dispatcher.set_focus(sender);
            if let Ok(encoded) = api::ipc::encode(&InputResponse::Ok, &mut resp_buf) {
                sys_send(sender, encoded);
            }
        }
        Ok(InputRequest::GetFocus) => {
            let focused = dispatcher.focus() as u32;
            if let Ok(encoded) = api::ipc::encode(&InputResponse::Focus(focused), &mut resp_buf) {
                sys_send(sender, encoded);
            }
        }
        Ok(InputRequest::ClearFocus { cell_tid }) => {
            if dispatcher.focus() == cell_tid as usize {
                dispatcher.set_focus(0);
            }
            if let Ok(encoded) = api::ipc::encode(&InputResponse::Ok, &mut resp_buf) {
                sys_send(sender, encoded);
            }
        }
        Err(_) => {} // unknown message — drop silently
    }
}
