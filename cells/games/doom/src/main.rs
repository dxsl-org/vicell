//! DOOM port for ViCell — doomgeneric platform backend.
//!
//! Implements the 6 doomgeneric platform hooks as `#[no_mangle] extern "C"`
//! functions, wiring DOOM's rendering and input into the ViCell compositor
//! and input service.
//!
//! ## How to run
//! 1. Clone doomgeneric: `git clone https://github.com/ozkl/doomgeneric cells/apps/doom/src/c/doomgeneric`
//! 2. Place `doom1.wad` (shareware) on the ViCell FAT32 disk as `/doom1.wad`
//! 3. Build and spawn: the compositor must be running; keyboard focus is requested at init.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![allow(static_mut_refs)]   // single-task cell — no data race on SURFACE / KEY_QUEUE

extern crate alloc;

use alloc::vec::Vec;
use api::declare_manifest;
use api::input::{InputEvent, KeyState, KeySym};
use api::display::PixelFormat;
use ostd::display::{ViSurface, wait_for_compositor};
use ostd::input::{poll_events, request_focus};
use ostd::syscall::{sys_exit, sys_get_time};
use ostd::task::yield_now;

// DOOM resolution — must match DOOMGENERIC_RESX/RESY in build.rs
const DOOM_W: u32 = 320;
const DOOM_H: u32 = 200;

declare_manifest!(block_io = false, network = false, spawn = false);

// ─── doomgeneric public API ────────────────────────────────────────────────────

extern "C" {
    // Initialize DOOM engine (parses WAD, sets up renderer, calls DG_Init).
    fn doomgeneric_Create(argc: i32, argv: *const *const u8);
    // Run one game tick: input → game logic → DG_DrawFrame + DG_GetKey.
    fn doomgeneric_Tick();
}

// ─── Global state (set once in DG_Init, read in DG_DrawFrame / DG_GetKey) ────

static mut SURFACE: Option<ViSurface> = None;

// Ring buffer for pending DOOM key events: (pressed: bool, doomkey: u8)
static mut KEY_QUEUE: [KeyQueueEntry; 32] = [KeyQueueEntry { pressed: false, doomkey: 0 }; 32];
static mut KEY_HEAD: usize = 0;
static mut KEY_TAIL: usize = 0;

#[derive(Clone, Copy)]
struct KeyQueueEntry {
    pressed: bool,
    doomkey: u8,
}

// ─── ViCell entry ──────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn main() {
    // Pass `-iwad /doom1.wad` so D_DoomMain can find the WAD on the ViFS.
    let iwad  = b"-iwad\0";
    let wpath = b"/doom1.wad\0";
    let argv: [*const u8; 3] = [
        b"doom\0".as_ptr(),
        iwad.as_ptr(),
        wpath.as_ptr(),
    ];

    // Initialize engine + start game loop (doomgeneric_Tick never returns).
    unsafe {
        doomgeneric_Create(3, argv.as_ptr());
        loop { doomgeneric_Tick(); }
    }
}

// ─── DG_Init ──────────────────────────────────────────────────────────────────

/// Called once by doomgeneric before the game loop starts.
/// Creates a 320×200 compositor surface and requests keyboard focus.
#[no_mangle]
pub unsafe extern "C" fn DG_Init() {
    let comp_tid = wait_for_compositor();
    match ViSurface::create(comp_tid, DOOM_W, DOOM_H, PixelFormat::Bgra8888) {
        Ok(surf) => { SURFACE = Some(surf); }
        Err(_)   => { sys_exit(1); }
    }
    // Spin until input service is registered and grants focus.
    // During boot race this can take a few ticks.
    while !request_focus() {
        yield_now();
    }
}

// ─── DG_DrawFrame ─────────────────────────────────────────────────────────────

/// Called by doomgeneric after each rendered frame.
/// Copies the 320×200 BGRA framebuffer to the compositor Grant surface.
///
/// doomgeneric stores screen data as 32-bit ARGB values (0xAARRGGBB).
/// In little-endian memory the bytes are [B, G, R, A] — matching Bgra8888.
#[no_mangle]
pub unsafe extern "C" fn DG_DrawFrame() {
    extern "C" {
        // DG_ScreenBuffer: allocated by doomgeneric_Create, 320×200 uint32_t
        static DG_ScreenBuffer: *mut u32;
    }
    let surf = match SURFACE.as_mut() {
        Some(s) => s,
        None    => return,
    };
    let byte_count = (DOOM_W * DOOM_H * 4) as usize;
    let src = core::slice::from_raw_parts(DG_ScreenBuffer as *const u8, byte_count);
    let dst = surf.pixels_mut();
    dst[..byte_count].copy_from_slice(src);
    surf.damage_all();
}

// ─── DG_GetKey ────────────────────────────────────────────────────────────────

/// Non-blocking key poll consumed by doomgeneric's event loop.
/// Returns 1 if an event is available, 0 if the queue is empty.
/// `*pressed` = 1 for press, 0 for release. `*doomkey` = DOOM key code.
#[no_mangle]
pub unsafe extern "C" fn DG_GetKey(pressed: *mut i32, doomkey: *mut u8) -> i32 {
    // Drain any new InputEvents from the input service into our ring buffer.
    let events: Vec<InputEvent> = poll_events(16);
    for ev in events {
        if let InputEvent::Key(ke) = ev {
            let dk = keysym_to_doom(ke.keysym, ke.character);
            if dk == 0 { continue; }
            let p = ke.state == KeyState::Pressed || ke.state == KeyState::Repeated;
            let tail = KEY_TAIL;
            let next = (tail + 1) % 32;
            if next != KEY_HEAD {
                KEY_QUEUE[tail] = KeyQueueEntry { pressed: p, doomkey: dk };
                KEY_TAIL = next;
            }
        }
    }

    if KEY_HEAD == KEY_TAIL {
        return 0;
    }
    let entry = KEY_QUEUE[KEY_HEAD];
    KEY_HEAD = (KEY_HEAD + 1) % 32;
    *pressed = entry.pressed as i32;
    *doomkey = entry.doomkey;
    1
}

// ─── DG_GetTicksMs ────────────────────────────────────────────────────────────

/// Monotonic millisecond counter.  sys_get_time() returns 10 MHz ticks.
#[no_mangle]
pub unsafe extern "C" fn DG_GetTicksMs() -> u32 {
    (sys_get_time() / 10_000) as u32
}

// ─── DG_SleepMs ───────────────────────────────────────────────────────────────

/// Busy-yield sleep.  Accurate enough for DOOM's 35 Hz game tick (~28 ms/frame).
#[no_mangle]
pub unsafe extern "C" fn DG_SleepMs(ms: u32) {
    let deadline = sys_get_time() + ms as u64 * 10_000;
    while sys_get_time() < deadline {
        yield_now();
    }
}

// ─── DG_Quit ──────────────────────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn DG_Quit() {
    sys_exit(0);
}

// ─── DG_SetWindowTitle ────────────────────────────────────────────────────────

/// No-op: ViCell surfaces don't have a title bar.
#[no_mangle]
pub unsafe extern "C" fn DG_SetWindowTitle(_title: *const u8) {}

// ─── mkdir stub ───────────────────────────────────────────────────────────────

/// DOOM calls mkdir() to create save-game directories. We have no real FS
/// directory creation yet — return 0 (success) so DOOM continues; actual
/// save files will fail gracefully when opened.
#[no_mangle]
pub unsafe extern "C" fn mkdir(_path: *const u8, _mode: u32) -> i32 {
    0
}

// ─── Key translation ─────────────────────────────────────────────────────────

// DOOM key constants (from doomgeneric.h / doomkeys.h).
// dead_code: STRAFE/FIRE/PAUSE become active once Modifier key mapping is wired.
const DOOM_KEY_RIGHTARROW: u8 = 0xae;
const DOOM_KEY_LEFTARROW:  u8 = 0xac;
const DOOM_KEY_UPARROW:    u8 = 0xad;
const DOOM_KEY_DOWNARROW:  u8 = 0xaf;
#[allow(dead_code)] const DOOM_KEY_STRAFE_L:   u8 = 0xa0;
#[allow(dead_code)] const DOOM_KEY_STRAFE_R:   u8 = 0xa1;
const DOOM_KEY_USE:        u8 = 0xa2;  // Space / Enter
#[allow(dead_code)] const DOOM_KEY_FIRE:       u8 = 0xa3;  // Ctrl
const DOOM_KEY_ESCAPE:     u8 = 27;
const DOOM_KEY_ENTER:      u8 = 13;
const DOOM_KEY_TAB:        u8 = 9;
const DOOM_KEY_F1:         u8 = 0x80 + 0x3b;
const DOOM_KEY_F2:         u8 = 0x80 + 0x3c;
const DOOM_KEY_F3:         u8 = 0x80 + 0x3d;
const DOOM_KEY_F4:         u8 = 0x80 + 0x3e;
const DOOM_KEY_F5:         u8 = 0x80 + 0x3f;
const DOOM_KEY_F6:         u8 = 0x80 + 0x40;
const DOOM_KEY_F7:         u8 = 0x80 + 0x41;
const DOOM_KEY_F8:         u8 = 0x80 + 0x42;
const DOOM_KEY_F9:         u8 = 0x80 + 0x43;
const DOOM_KEY_F10:        u8 = 0x80 + 0x44;
const DOOM_KEY_F11:        u8 = 0x80 + 0x57;
const DOOM_KEY_F12:        u8 = 0x80 + 0x58;
#[allow(dead_code)] const DOOM_KEY_PAUSE:      u8 = 0xff;

fn keysym_to_doom(sym: KeySym, character: u32) -> u8 {
    match sym {
        KeySym::Escape    => DOOM_KEY_ESCAPE,
        KeySym::Return    => DOOM_KEY_USE,
        KeySym::Tab       => DOOM_KEY_TAB,
        KeySym::Up        => DOOM_KEY_UPARROW,
        KeySym::Down      => DOOM_KEY_DOWNARROW,
        KeySym::Left      => DOOM_KEY_LEFTARROW,
        KeySym::Right     => DOOM_KEY_RIGHTARROW,
        KeySym::F1        => DOOM_KEY_F1,
        KeySym::F2        => DOOM_KEY_F2,
        KeySym::F3        => DOOM_KEY_F3,
        KeySym::F4        => DOOM_KEY_F4,
        KeySym::F5        => DOOM_KEY_F5,
        KeySym::F6        => DOOM_KEY_F6,
        KeySym::F7        => DOOM_KEY_F7,
        KeySym::F8        => DOOM_KEY_F8,
        KeySym::F9        => DOOM_KEY_F9,
        KeySym::F10       => DOOM_KEY_F10,
        KeySym::F11       => DOOM_KEY_F11,
        KeySym::F12       => DOOM_KEY_F12,
        KeySym::Printable => {
            // Map printable to ASCII doom key.
            // DOOM checks lowercase keys; space=use, ctrl=fire(mapped via Modifiers),
            // alt=strafe (not yet handled — use as STRAFE_L for now).
            match character as u8 {
                b' '           => DOOM_KEY_USE,
                b'\r' | b'\n'  => DOOM_KEY_ENTER,
                b'\t'          => DOOM_KEY_TAB,
                c @ b'a'..=b'z' => c,
                c @ b'A'..=b'Z' => c + 32, // DOOM expects lowercase
                c @ b'0'..=b'9' => c,
                c              => c,
            }
        }
        KeySym::Unknown  |
        KeySym::Backspace |
        KeySym::Delete    |
        KeySym::Insert    |
        KeySym::Home      |
        KeySym::End       |
        KeySym::PageUp    |
        KeySym::PageDown  => 0,
    }
}
