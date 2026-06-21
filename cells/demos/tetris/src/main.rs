//! Tetris — native Rust implementation for ViCell.
//!
//! Pure no_std Rust: game logic in game.rs, rendering in draw.rs.
//! Uses ostd ViSurface + FONT8X8 for graphics, ostd input for keyboard.

#![no_std]
#![no_main]

extern crate alloc;

mod game;
mod draw;

use api::declare_manifest;
use api::input::{InputEvent, KeyState, KeySym};
use api::display::PixelFormat;
use ostd::display::{ViSurface, wait_for_compositor};
use ostd::input::{poll_events, request_focus};
use ostd::syscall::{sys_exit, sys_get_time};
use ostd::task::yield_now;

declare_manifest!(block_io = false, network = false, spawn = false);

fn ticks_ms() -> u32 { (sys_get_time() / 10_000) as u32 }

#[no_mangle]
pub extern "C" fn main() {
    let comp = wait_for_compositor();
    let mut surf = match ViSurface::create(
        comp,
        api::display::FALLBACK_WIDTH,
        api::display::FALLBACK_HEIGHT,
        PixelFormat::Bgra8888,
    ) {
        Ok(s) => s,
        Err(_) => { sys_exit(1); }
    };
    surf.raise();

    while !request_focus() { yield_now(); }

    let mut game = game::Game::new(ticks_ms());
    draw::render(&mut surf, &game);

    let mut last_tick = ticks_ms();
    let mut last_key_time = 0u32;
    const KEY_REPEAT_MS: u32 = 80;

    loop {
        if game.over {
            // Escape = exit; any other key = restart
            let evs = poll_events(4);
            for ev in evs {
                if let InputEvent::Key(ke) = ev {
                    if ke.state == KeyState::Pressed {
                        if ke.keysym == KeySym::Escape {
                            sys_exit(0);
                        }
                        game = game::Game::new(ticks_ms());
                        draw::render(&mut surf, &game);
                        last_tick = ticks_ms();
                    }
                }
            }
            yield_now();
            continue;
        }

        let now = ticks_ms();
        let dt = now.wrapping_sub(last_tick);
        let can_repeat = now.wrapping_sub(last_key_time) >= KEY_REPEAT_MS;

        let evs = poll_events(8);
        let mut redraw = false;

        for ev in evs {
            if let InputEvent::Key(ke) = ev {
                match ke.state {
                    KeyState::Pressed | KeyState::Repeated => {
                        let first = ke.state == KeyState::Pressed;
                        if first || can_repeat {
                            // Space (char ' ') or Return = hard drop; Up = rotate
                        let ch = ke.char();
                        let is_space = ch == Some(' ');
                        match ke.keysym {
                                KeySym::Left   => { game.move_left();  redraw = true; }
                                KeySym::Right  => { game.move_right(); redraw = true; }
                                KeySym::Up     => { game.rotate();     redraw = true; }
                                KeySym::Down   => { game.soft_drop();  redraw = true; }
                                KeySym::Return => { game.hard_drop();  redraw = true; }
                                KeySym::Escape => { sys_exit(0); }
                                KeySym::Printable => match ch {
                                    Some(' ')                     => { game.hard_drop();  redraw = true; }
                                    Some('a') | Some('A')
                                    | Some('j') | Some('J')       => { game.move_left();  redraw = true; }
                                    Some('d') | Some('D')
                                    | Some('l') | Some('L')       => { game.move_right(); redraw = true; }
                                    Some('w') | Some('W')
                                    | Some('i') | Some('I')       => { game.rotate();     redraw = true; }
                                    Some('s') | Some('S')
                                    | Some('k') | Some('K')       => { game.soft_drop();  redraw = true; }
                                    _ => { let _ = is_space; }
                                }
                                _ => {}
                            }
                            if redraw { last_key_time = now; }
                        }
                    }
                    _ => {}
                }
            }
        }

        if dt > 0 {
            let was_over = game.over;
            game.tick(dt);
            last_tick = now;
            if game.over != was_over || dt >= 16 { redraw = true; }
        }

        if redraw { draw::render(&mut surf, &game); }
        else { yield_now(); }
    }
}
