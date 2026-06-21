//! Tetris port for ViCell — Banaxi-Tech/Tetris-OS platform backend.
//!
//! Implements the ViCell platform callbacks required by vicell_platform.c:
//!   - `vicell_get_ticks_ms`  monotonic millisecond counter
//!   - `vicell_poll_key`      consume one queued key event (KEY_* int)
//!   - `vicell_surface_ptr`   raw pointer to the live BGRA compositor surface
//!   - `vicell_flush`         signal a full-surface damage to the compositor
//!   - `vicell_draw_char`     render one 8×8 glyph (scaled 3×) using ostd FONT8X8
//!
//! ## Setup
//! 1. Clone Tetris-OS: `git clone https://github.com/Banaxi-Tech/Tetris-OS cells/games/tetris-c/src/c/tetris-os`
//! 2. Verify the game entry name (see vicell_platform.c bottom section).
//! 3. Build and run: the compositor must be active; compositor+tetris share keyboard focus.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![allow(static_mut_refs)]  // single-task cell — no data race on SURFACE / KEY_QUEUE

extern crate alloc;

use alloc::vec::Vec;
use api::declare_manifest;
use api::input::{InputEvent, KeyState, KeySym};
use api::display::PixelFormat;
use ostd::display::{ViSurface, wait_for_compositor};
use ostd::font::FONT8X8;
use ostd::input::{poll_events, request_focus};
use ostd::syscall::{sys_exit, sys_get_time};
use ostd::task::yield_now;

// ── Screen geometry ───────────────────────────────────────────────────────
// VGA canvas: 320×200.  3× nearest-neighbour scale → 960×600.
// Centred in 1024×768: X offset = (1024-960)/2 = 32, Y = (768-600)/2 = 84.
const SURF_W: u32 = api::display::FALLBACK_WIDTH;   // 1024
const SURF_H: u32 = api::display::FALLBACK_HEIGHT;  // 768
const SCALE:  u32 = 3;

declare_manifest!(block_io = false, network = false, spawn = false);

// ── Compositor surface ────────────────────────────────────────────────────
static mut SURFACE: Option<ViSurface> = None;

// ── Key event ring buffer ─────────────────────────────────────────────────
// KEY_* codes must match keyboard.h from Banaxi-Tech/Tetris-OS.
// If the _Static_asserts in vicell_platform.c fail, update these constants.
const KEY_NONE:   i32 = 0;
const KEY_LEFT:   i32 = 1;
const KEY_RIGHT:  i32 = 2;
const KEY_UP:     i32 = 3;
const KEY_DOWN:   i32 = 4;
const KEY_ENTER:  i32 = 5;
const KEY_ESCAPE: i32 = 6;

static mut KEY_QUEUE: [i32; 16] = [KEY_NONE; 16];
static mut KEY_HEAD:  usize = 0;
static mut KEY_TAIL:  usize = 0;

unsafe fn enqueue_key(k: i32) {
    let tail = KEY_TAIL;
    let next = (tail + 1) % 16;
    if next != KEY_HEAD {
        KEY_QUEUE[tail] = k;
        KEY_TAIL = next;
    }
}

// ── ViCell entry ──────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn main() {
    let comp_tid = wait_for_compositor();
    unsafe {
        match ViSurface::create(comp_tid, SURF_W, SURF_H, PixelFormat::Bgra8888) {
            Ok(surf) => {
                surf.raise();
                SURFACE = Some(surf);
            }
            Err(_) => { sys_exit(1); }
        }
        while !request_focus() {
            yield_now();
        }
        tetris_cell_run();
    }
    sys_exit(0);
}

// ── C game entry point ────────────────────────────────────────────────────
// tetris_cell_run() is defined in vicell_platform.c and calls tetris_run()
// (from tetris.c).  If tetris.c uses a different symbol, update vicell_platform.c.
extern "C" {
    fn tetris_cell_run();
}

// ── Platform callbacks (called from vicell_platform.c) ────────────────────

/// Monotonic millisecond counter.  sys_get_time() returns 10 MHz ticks.
#[no_mangle]
pub unsafe extern "C" fn vicell_get_ticks_ms() -> u32 {
    (sys_get_time() / 10_000) as u32
}

/// Dequeue one pending key event.  Drains new ViCell input events first.
/// Returns KEY_NONE (0) when the queue is empty.
#[no_mangle]
pub unsafe extern "C" fn vicell_poll_key() -> i32 {
    let events: Vec<InputEvent> = poll_events(8);
    for ev in events {
        if let InputEvent::Key(ke) = ev {
            let k = match ke.state {
                KeyState::Pressed | KeyState::Repeated => keysym_to_tetris(ke.keysym),
                _ => continue,
            };
            if k != KEY_NONE {
                enqueue_key(k);
            }
        }
    }
    if KEY_HEAD == KEY_TAIL {
        return KEY_NONE;
    }
    let k = KEY_QUEUE[KEY_HEAD];
    KEY_HEAD = (KEY_HEAD + 1) % 16;
    k
}

/// Raw pointer to the live BGRA compositor surface buffer.
/// Returns null if the surface is not yet initialised (should not happen during gameplay).
#[no_mangle]
pub unsafe extern "C" fn vicell_surface_ptr() -> *mut u32 {
    match SURFACE.as_mut() {
        Some(s) => s.pixels_mut().as_mut_ptr() as *mut u32,
        None    => core::ptr::null_mut(),
    }
}

/// Signal full-surface damage to the compositor (triggers a repaint).
#[no_mangle]
pub unsafe extern "C" fn vicell_flush() {
    if let Some(ref s) = SURFACE {
        s.damage_all();
    }
}

/// Render one 8×8 glyph at scaled screen coordinate (x, y) using FONT8X8.
///
/// `x`, `y` — top-left corner in surface pixels (already scaled + offset-adjusted).
/// `c`      — ASCII byte (0x20–0x7E; other values render as space).
/// `bgra`   — BGRA8888 colour as little-endian u32: (A<<24)|(R<<16)|(G<<8)|B.
///
/// Each VGA pixel is expanded to SCALE×SCALE surface pixels (nearest-neighbour).
/// Font bit order: MSB = leftmost column (mask & (0x80 >> col) tests column col).
#[no_mangle]
pub unsafe extern "C" fn vicell_draw_char(x: u32, y: u32, c: u8, bgra: u32) {
    let surf = match SURFACE.as_mut() {
        Some(s) => s,
        None    => return,
    };
    let sw    = surf.width() as i32;
    let sh    = surf.height() as i32;
    let stride = surf.stride(); // bytes per row
    let pixels = surf.pixels_mut();

    let idx = if c >= 0x20 && c <= 0x7E { (c - 0x20) as usize } else { 0 };
    let glyph = &FONT8X8[idx];

    // Unpack BGRA bytes to write into the pixel buffer
    let pb = (bgra & 0xFF) as u8;
    let pg = ((bgra >> 8) & 0xFF) as u8;
    let pr = ((bgra >> 16) & 0xFF) as u8;
    let pa = ((bgra >> 24) & 0xFF) as u8;

    for row in 0..8_i32 {
        let mask = glyph[row as usize];
        if mask == 0 { continue; }
        for col in 0..8_i32 {
            if mask & (0x80u8 >> col as u32) == 0 { continue; }
            for dy in 0..SCALE as i32 {
                let py = y as i32 + row * SCALE as i32 + dy;
                if py < 0 || py >= sh { continue; }
                for dx in 0..SCALE as i32 {
                    let px = x as i32 + col * SCALE as i32 + dx;
                    if px < 0 || px >= sw { continue; }
                    let off = py as usize * stride + px as usize * 4;
                    if off + 4 <= pixels.len() {
                        pixels[off]     = pb;
                        pixels[off + 1] = pg;
                        pixels[off + 2] = pr;
                        pixels[off + 3] = pa;
                    }
                }
            }
        }
    }
}

// ── Key translation ───────────────────────────────────────────────────────

fn keysym_to_tetris(sym: KeySym) -> i32 {
    match sym {
        KeySym::Left     => KEY_LEFT,
        KeySym::Right    => KEY_RIGHT,
        KeySym::Up       => KEY_UP,
        KeySym::Down     => KEY_DOWN,
        KeySym::Return   => KEY_ENTER,
        KeySym::Escape   => KEY_ESCAPE,
        KeySym::Printable => KEY_NONE, // letter keys not used in Tetris
        _                => KEY_NONE,
    }
}
