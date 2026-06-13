//! Compositor-side input event handler.
//!
//! When the compositor registers as the input service's focus endpoint, all
//! dispatched `InputEvent`s arrive here prefixed with `INPUT_EVENT_OPCODE (0x10)`.
//! This module:
//!   - Forwards key events to the focused surface owner's cell.
//!   - Tracks the logical mouse cursor position.
//!   - On mouse move, unions the old and new 16×16 cursor rects into
//!     `pending_dirty` so the compositor repaints both positions (no trail).
//!   - On left-click, hit-tests the surface z-stack and updates keyboard focus.

use api::display::Rect;
use api::ipc::{InputRequest, IPC_BUF_SIZE};
use api::syscall::service;
use ostd::syscall::{sys_lookup_service, sys_recv, sys_send};

use crate::cursor_sprite::{CURSOR_H, CURSOR_W};
use crate::surface_table::SurfaceTable;
use crate::z_order::ZOrder;

/// Opcode prefix byte used by the input service dispatcher when sending events.
/// Must match `cells/services/input/src/dispatcher::INPUT_EVENT_OPCODE`.
const INPUT_EVENT_OPCODE: u8 = 0x10;

/// Total IPC frame size for one input event (opcode byte + 64-byte payload).
const INPUT_FRAME_LEN: usize = 65;

/// Conventional TID of the shell cell — used as the default keyboard focus
/// before any surface claims it. Matches `dispatcher::DEFAULT_FOCUS_ENDPOINT`.
const DEFAULT_SHELL_TID: usize = 3;

/// Input routing and mouse position state owned by the compositor.
pub struct InputState {
    /// TID of the input service cell (0 = not yet connected).
    pub input_tid: usize,
    /// Logical mouse cursor position (updated from MouseMove events).
    pub mouse_x: i32,
    pub mouse_y: i32,
    /// TID of the cell currently receiving keyboard events.
    focused_owner: usize,
}

impl InputState {
    pub fn new() -> Self {
        Self { input_tid: 0, mouse_x: 0, mouse_y: 0, focused_owner: DEFAULT_SHELL_TID }
    }
}

/// Look up a service, yielding until it becomes available.
fn wait_for_service(id: u16) -> usize {
    loop {
        if let Some(tid) = sys_lookup_service(id) {
            return tid;
        }
        ostd::task::yield_now();
    }
}

/// Register the compositor as the input focus so that all events are routed here.
///
/// Blocks briefly at startup until both the input service and compositor's own
/// TID are registered in the service table (init does both before yielding).
pub fn connect_to_input(state: &mut InputState) {
    let input_tid = wait_for_service(service::INPUT);
    let own_tid   = wait_for_service(service::COMPOSITOR);
    state.input_tid = input_tid;

    let mut req_buf = [0u8; IPC_BUF_SIZE];
    let req = InputRequest::SetFocus { cell_tid: own_tid as u32 };
    if let Ok(encoded) = api::ipc::encode(&req, &mut req_buf) {
        sys_send(input_tid, encoded);
        // Drain the InputResponse::Ok so input service is not left blocked.
        let mut resp_buf = [0u8; IPC_BUF_SIZE];
        let _ = sys_recv(0, &mut resp_buf);
    }
}

/// Dispatch a raw IPC buffer received from the input service.
///
/// Only called when `sender == state.input_tid`.
/// `buf[0]` must equal `INPUT_EVENT_OPCODE`.
///
/// On `MouseMove` (discriminant 1), unions the old cursor rect and the new
/// cursor rect into `pending_dirty` so the compositor repaints both positions.
pub fn handle_input_event(
    buf: &[u8; 512],
    state: &mut InputState,
    table: &SurfaceTable,
    z_order: &ZOrder,
    pending_dirty: &mut Option<Rect>,
) {
    if buf[0] != INPUT_EVENT_OPCODE { return; }
    match buf[1] {
        0 => forward_key(buf, state),
        1 => update_cursor(buf, state, pending_dirty),
        2 => on_mouse_button(buf, state, table, z_order),
        _ => {}
    }
}

/// Re-send the key-event frame to the focused surface owner.
fn forward_key(buf: &[u8; 512], state: &InputState) {
    if state.focused_owner != 0 {
        sys_send(state.focused_owner, &buf[..INPUT_FRAME_LEN]);
    }
}

/// Build a screen-space rect covering the cursor sprite at `(x, y)`.
#[inline]
fn cursor_rect(x: i32, y: i32) -> Rect {
    Rect { x, y, w: CURSOR_W, h: CURSOR_H }
}

/// Update logical mouse position from a MouseMove payload.
///
/// MouseMove layout (buf offsets after the opcode byte):
///   buf[2..6] = x (i32 LE), buf[6..10] = y (i32 LE)
///
/// Unions old cursor rect + new cursor rect into `pending_dirty` so the
/// compositor repaints both positions on the next frame (eliminates trail).
/// Emits `[compositor] cursor at X,Y` for the Phase 04 integration test probe.
fn update_cursor(buf: &[u8; 512], state: &mut InputState, pending_dirty: &mut Option<Rect>) {
    let old_rect = cursor_rect(state.mouse_x, state.mouse_y);

    state.mouse_x = i32::from_le_bytes([buf[2], buf[3], buf[4], buf[5]]);
    state.mouse_y = i32::from_le_bytes([buf[6], buf[7], buf[8], buf[9]]);

    let new_rect = cursor_rect(state.mouse_x, state.mouse_y);
    let combined = old_rect.union(&new_rect);
    *pending_dirty = Some(match pending_dirty.take() {
        Some(acc) => acc.union(&combined),
        None => combined,
    });

    // One-line probe consumed by the Phase 04 integration test.
    ostd::println!("[compositor] cursor at {},{}", state.mouse_x, state.mouse_y);
}

/// On left-button press, find the topmost surface under the cursor and update focus.
///
/// MouseButton layout: buf[2] = button (0=Left), buf[3] = state (1=Pressed).
fn on_mouse_button(
    buf: &[u8; 512],
    state: &mut InputState,
    table: &SurfaceTable,
    z_order: &ZOrder,
) {
    if buf[2] != 0 || buf[3] != 1 { return; } // only left-press
    if let Some(owner) = hit_test(state.mouse_x, state.mouse_y, table, z_order) {
        state.focused_owner = owner;
    }
}

/// Return the owner TID of the topmost surface containing `(x, y)`, or `None`.
fn hit_test(x: i32, y: i32, table: &SurfaceTable, z_order: &ZOrder) -> Option<usize> {
    for cap in z_order.iter_top_to_bottom() {
        if let Some(s) = table.get(cap) {
            let r = s.screen_rect();
            if x >= r.x && x < r.x + r.w as i32 && y >= r.y && y < r.y + r.h as i32 {
                return Some(s.owner);
            }
        }
    }
    None
}
