#![no_std]
#![no_main]
// #[no_mangle] on main() requires removing global forbid — all submodules stay unsafe-free.

//! Compositor Service Cell.
//!
//! Manages a z-ordered set of cell surfaces, blends them into a screen
//! framebuffer, and flushes dirty regions to the VirtIO GPU via the
//! `GpuFlush` kernel syscall.
//!
//! Input routing: on startup the compositor registers as the input service's
//! focus endpoint.  All `InputEvent` IPC frames are dispatched to
//! `input_handler`, which forwards keyboard events to the focused surface
//! owner and updates the cursor position on mouse move.

extern crate alloc;

mod cursor_sprite;
mod input_handler;
mod render;
mod surface_table;
mod z_order;

use api::display::{AttachGrant, DamageNotify, PixelFormat, compositor_ops, Rect};
use input_handler::{InputState, connect_to_input, handle_input_event};
use ostd::io::println;
use ostd::syscall::{sys_grant_slice, sys_recv, sys_send, SyscallResult};
use render::{render_frame, ScreenFb};
use surface_table::SurfaceTable;
use z_order::ZOrder;

/// IPC opcode prefix byte identifying an input event from the input service.
/// Must match `input_handler::INPUT_EVENT_OPCODE` (0x10).
const INPUT_EVENT_OPCODE: u8 = 0x10;

#[no_mangle]
pub fn main() {
    println("[compositor] Compositor v0.2: software blending, VirtIO GPU, input routing");

    let (w, h)   = render::default_screen_size();
    let mut fb      = ScreenFb::new(w, h);
    let mut table   = SurfaceTable::new();
    let mut z_order = ZOrder::new();
    let mut input        = InputState::new();
    // Compositor-initiated repaint region: set by cursor moves, surface destroy/raise.
    // Consumed (taken) on each render_frame call.
    let mut pending_dirty: Option<Rect> = None;

    // Register as input focus so keyboard + mouse events flow to us.
    connect_to_input(&mut input);

    let mut buf = [0u8; 512];

    loop {
        match sys_recv(0, &mut buf) {
            SyscallResult::Ok(sender) if sender > 0 => {
                if input.input_tid != 0 && sender == input.input_tid
                    && buf[0] == INPUT_EVENT_OPCODE
                {
                    // On MouseMove, update_cursor sets pending_dirty = union(old, new)
                    // so the frame is repainted at the interval tick.
                    handle_input_event(
                        &buf, &mut input, &table, &z_order, &mut pending_dirty,
                    );
                } else {
                    handle_message(&buf, sender, &mut table, &mut z_order,
                                   &mut pending_dirty);
                }
            }
            _ => {
                ostd::task::yield_now();
            }
        }

        // Damage-driven: render when a surface reports damage or cursor/compositor dirty.
        if table.has_damage() || pending_dirty.is_some() {
            let _ = render_frame(
                &mut fb, &mut table, &z_order,
                pending_dirty.take(),
                input.mouse_x, input.mouse_y,
            );
        }
    }
}

/// Dispatch one IPC message from a consumer cell.
fn handle_message(
    buf: &[u8; 512],
    sender: usize,
    table: &mut SurfaceTable,
    z_order: &mut ZOrder,
    pending_dirty: &mut Option<api::display::Rect>,
) {
    if buf.is_empty() { return; }
    #[allow(deprecated)] // WRITE_PIXELS kept for legacy clients; new code uses ATTACH_GRANT
    match buf[0] {
        compositor_ops::CREATE_SURFACE => {
            if buf.len() < 9 { return; }
            let sw = u32::from_le_bytes([buf[1], buf[2], buf[3], buf[4]]);
            let sh = u32::from_le_bytes([buf[5], buf[6], buf[7], buf[8]]);
            match table.create(0, 0, sw, sh, sender) {
                Ok(cap) => {
                    z_order.push(cap);
                    sys_send(sender, &cap.to_le_bytes());
                }
                Err(_) => {
                    sys_send(sender, &0u64.to_le_bytes());
                }
            }
        }
        compositor_ops::WRITE_PIXELS => {
            if buf.len() < 25 { return; }
            let cap = u64::from_le_bytes([buf[1],buf[2],buf[3],buf[4],buf[5],buf[6],buf[7],buf[8]]);
            let x  = i32::from_le_bytes([buf[9], buf[10],buf[11],buf[12]]);
            let y  = i32::from_le_bytes([buf[13],buf[14],buf[15],buf[16]]);
            let pw = u32::from_le_bytes([buf[17],buf[18],buf[19],buf[20]]);
            let ph = u32::from_le_bytes([buf[21],buf[22],buf[23],buf[24]]);
            if let Some(s) = table.get_mut(cap) {
                s.write_pixels(x, y, pw, ph, &buf[25..]);
            }
        }
        compositor_ops::DAMAGE_SURFACE => {
            if buf.len() < 25 { return; }
            let cap = u64::from_le_bytes([buf[1],buf[2],buf[3],buf[4],buf[5],buf[6],buf[7],buf[8]]);
            let x  = i32::from_le_bytes([buf[9], buf[10],buf[11],buf[12]]);
            let y  = i32::from_le_bytes([buf[13],buf[14],buf[15],buf[16]]);
            let dw = u32::from_le_bytes([buf[17],buf[18],buf[19],buf[20]]);
            let dh = u32::from_le_bytes([buf[21],buf[22],buf[23],buf[24]]);
            if let Some(s) = table.get_mut(cap) {
                use api::display::Rect;
                let new_dmg = Rect { x, y, w: dw, h: dh };
                s.damage = Some(match s.damage { Some(d) => d.union(&new_dmg), None => new_dmg });
            }
        }
        compositor_ops::MOVE_SURFACE => {
            if buf.len() < 17 { return; }
            let cap = u64::from_le_bytes([buf[1],buf[2],buf[3],buf[4],buf[5],buf[6],buf[7],buf[8]]);
            let x = i32::from_le_bytes([buf[9], buf[10],buf[11],buf[12]]);
            let y = i32::from_le_bytes([buf[13],buf[14],buf[15],buf[16]]);
            if let Some(s) = table.get_mut(cap) {
                s.x = x; s.y = y;
                let (sw, sh) = (s.w, s.h);
                s.damage = Some(api::display::Rect { x: 0, y: 0, w: sw, h: sh });
            }
        }
        compositor_ops::RAISE_SURFACE => {
            if buf.len() < 9 { return; }
            let cap = u64::from_le_bytes([buf[1],buf[2],buf[3],buf[4],buf[5],buf[6],buf[7],buf[8]]);
            z_order.raise(cap);
            // Mark full-surface damage so the new z-order is visible on the next render.
            if let Some(s) = table.get_mut(cap) {
                let (sw, sh) = (s.w, s.h);
                let full = api::display::Rect { x: 0, y: 0, w: sw, h: sh };
                s.damage = Some(match s.damage { Some(d) => d.union(&full), None => full });
            }
        }
        compositor_ops::DESTROY_SURFACE => {
            if buf.len() < 9 { return; }
            let cap = u64::from_le_bytes([buf[1],buf[2],buf[3],buf[4],buf[5],buf[6],buf[7],buf[8]]);
            // Save the screen rect before removal so we can repaint the uncovered area.
            let freed_rect = table.get(cap).map(|s| s.screen_rect());
            z_order.remove(cap);
            let _ = table.remove(cap);
            sys_send(sender, b"\x00");
            // Queue the freed area as compositor-owned dirty so surfaces beneath
            // are re-blended even if they haven't sent a DamageNotify.
            if let Some(r) = freed_rect {
                *pending_dirty = Some(match pending_dirty.take() {
                    Some(acc) => acc.union(&r),
                    None => r,
                });
            }
        }
        compositor_ops::ATTACH_GRANT => {
            // Wire format: AttachGrant (24 bytes).
            // App must have called sys_grant_share(reg_id, comp_tid, 0 /*ReadOnly*/) first.
            if buf.len() < 24 { return; }
            let Ok(b24) = <&[u8; 24]>::try_from(&buf[..24]) else { return };
            let ag  = AttachGrant::decode(b24);
            let cap = ag.cap as u64;
            // Ownership check: only the surface creator may attach a Grant.
            if let Some(s) = table.get_mut(cap) {
                if s.owner != sender {
                    sys_send(sender, b"\x00");
                    return;
                }
                match sys_grant_slice(ag.reg_id as usize) {
                    Some(ptr) => {
                        s.attach_grant(ptr as *const u8, ag.reg_id as usize,
                                       ag.width, ag.height,
                                       PixelFormat::from_u8(ag.fmt));
                        sys_send(sender, b"\x01"); // OK
                    }
                    None => {
                        sys_send(sender, b"\x00"); // grant not shared or bad id
                    }
                }
            } else {
                sys_send(sender, b"\x00");
            }
        }
        compositor_ops::DAMAGE_NOTIFY => {
            // Wire format: DamageNotify (24 bytes).  Fire-and-forget — no reply.
            if buf.len() < 24 { return; }
            let Ok(b24) = <&[u8; 24]>::try_from(&buf[..24]) else { return };
            let dn  = DamageNotify::decode(b24);
            let cap = dn.cap as u64;
            if let Some(s) = table.get_mut(cap) {
                if s.owner == sender {
                    s.damage = Some(match s.damage {
                        Some(d) => d.union(&dn.rect),
                        None    => dn.rect,
                    });
                }
            }
        }
        compositor_ops::DETACH_GRANT => {
            // Payload: [opcode: u8, cap: u64 LE] — app about to free the Grant.
            if buf.len() < 9 { return; }
            let cap = u64::from_le_bytes([buf[1],buf[2],buf[3],buf[4],
                                          buf[5],buf[6],buf[7],buf[8]]);
            if let Some(s) = table.get_mut(cap) {
                if s.owner == sender {
                    s.detach_grant();
                }
            }
            sys_send(sender, b"\x01");
        }
        compositor_ops::GET_SCREEN_SIZE => {
            let (sw, sh) = render::default_screen_size();
            let mut reply = [0u8; 8];
            reply[0..4].copy_from_slice(&sw.to_le_bytes());
            reply[4..8].copy_from_slice(&sh.to_le_bytes());
            sys_send(sender, &reply);
        }
        _ => {}
    }
}
