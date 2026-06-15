// SPDX-License-Identifier: MPL-2.0

//! Generic input event client for any Cell.
//!
//! Provides focus registration and non-blocking event polling without any
//! dependency on `libs/viui`. ViUI apps should use `viui::input_bridge`
//! which wraps this module and converts events to `viui::Event`.
//!
//! # Usage
//! ```no_run
//! use ostd::input::{request_focus, poll_events, InputEvent};
//!
//! // Once at startup:
//! while !request_focus() { ostd::task::yield_now(); }
//!
//! // Every tick:
//! for ev in poll_events(32) {
//!     if let InputEvent::Key(k) = ev { /* handle key */ }
//! }
//! ```

extern crate alloc;
use alloc::vec::Vec;

// Re-export api::input types so consumers can use `ostd::input::KeyState` etc.
pub use api::input::{InputEvent, KeyEvent, KeyState, KeySym, Modifiers, MouseButton};

use api::{
    input::{INPUT_EVENT_OPCODE, decode_event},
    ipc::{InputRequest, InputResponse, IPC_BUF_SIZE},
    syscall::service,
};
use crate::syscall::{sys_lookup_service, sys_recv, sys_send, sys_try_recv, SyscallResult};


/// Register this cell as the keyboard/mouse focus recipient.
///
/// Sends `InputRequest::SetFocus` to the input service. The service uses the
/// kernel-verified IPC sender TID — the `cell_tid` field is ignored, preventing
/// TID impersonation.
///
/// Returns `true` when focus is granted. Returns `false` when the input service
/// is not yet registered (boot race) — callers should retry with a yield.
pub fn request_focus() -> bool {
    let Some(input_tid) = sys_lookup_service(service::INPUT) else { return false };

    let mut req_buf = [0u8; IPC_BUF_SIZE];
    let req = InputRequest::SetFocus { cell_tid: 0 };
    let Ok(encoded) = api::ipc::encode(&req, &mut req_buf) else { return false };
    sys_send(input_tid, encoded);

    let mut resp_buf = [0u8; IPC_BUF_SIZE];
    match sys_recv(0, &mut resp_buf) {
        SyscallResult::Ok(_sender) => {
            matches!(api::ipc::decode::<InputResponse>(&resp_buf), Ok(InputResponse::Ok))
        }
        _ => false,
    }
}

/// Non-blocking drain of pending input events (up to `max` events).
///
/// Calls `sys_try_recv` in a loop; stops when the queue is empty or `max` is
/// reached. Messages from senders other than the input service are discarded.
/// Safe to call every frame — returns an empty `Vec` during the boot race when
/// the input service is not yet registered.
pub fn poll_events(max: usize) -> Vec<InputEvent> {
    let Some(input_tid) = sys_lookup_service(service::INPUT) else {
        return Vec::new();
    };

    let mut events = Vec::with_capacity(max.min(16));
    while events.len() < max {
        let mut buf = [0u8; 65];
        match sys_try_recv(usize::MAX, &mut buf) {
            SyscallResult::Ok(0) => break, // queue empty
            SyscallResult::Ok(sender) if sender == input_tid => {
                if let Some(ev) = parse_frame(&buf) {
                    events.push(ev);
                }
            }
            SyscallResult::Ok(_) => {} // unexpected sender — discard
            SyscallResult::Err(_) => break,
        }
    }
    events
}

/// Decode a 65-byte input-service IPC message into an `InputEvent`.
///
/// Returns `None` for messages with a wrong opcode or unsupported discriminant.
/// Exposed as `pub(crate)` so `ostd::app` can reuse it without re-exporting.
pub(crate) fn parse_frame(buf: &[u8]) -> Option<InputEvent> {
    if buf.len() < 2 || buf[0] != INPUT_EVENT_OPCODE {
        return None;
    }
    decode_event(&buf[1..])
}
