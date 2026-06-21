//! Tetris renderer — all drawing operations on a ViSurface.

use ostd::display::ViSurface;
use ostd::font::FONT8X8;
use crate::game::{
    Game, COLS, ROWS, piece_cells,
    BG_COLOR, BORDER_COLOR, TEXT_COLOR, PANEL_COLOR, GHOST_COLOR, PIECE_COLORS,
};

const CELL_PX: u32 = 30;

// Board top-left in surface pixels; score panel to the right.
const BX: u32 = 252;
const BY: u32 = 84;
const PX: u32 = BX + COLS as u32 * CELL_PX + 24; // panel X

pub fn render(surf: &mut ViSurface, game: &Game) {
    fill(surf, 0, 0, surf.width(), surf.height(), BG_COLOR);

    // Board border
    fill(surf, BX - 2, BY - 2, COLS as u32 * CELL_PX + 4, ROWS as u32 * CELL_PX + 4, BORDER_COLOR);
    fill(surf, BX, BY, COLS as u32 * CELL_PX, ROWS as u32 * CELL_PX, BG_COLOR);

    // Locked cells
    for r in 0..ROWS {
        for c in 0..COLS {
            let ci = game.board[r][c];
            if ci != 0 {
                draw_cell(surf, BX + c as u32 * CELL_PX, BY + r as u32 * CELL_PX, PIECE_COLORS[ci as usize - 1]);
            }
        }
    }

    if !game.over {
        // Ghost piece
        let gr = game.ghost_row();
        if gr != game.row {
            for (x, y) in piece_cells(game.kind, game.rot, game.col, gr) {
                if y >= 0 {
                    fill(surf,
                        BX + x as u32 * CELL_PX + 3,
                        BY + y as u32 * CELL_PX + 3,
                        CELL_PX - 6, CELL_PX - 6, GHOST_COLOR);
                }
            }
        }
        // Active piece
        let color = PIECE_COLORS[game.kind];
        for (x, y) in piece_cells(game.kind, game.rot, game.col, game.row) {
            if y >= 0 {
                draw_cell(surf, BX + x as u32 * CELL_PX, BY + y as u32 * CELL_PX, color);
            }
        }
    }

    // Score panel background
    fill(surf, PX - 4, BY - 2, 160, ROWS as u32 * CELL_PX + 4, PANEL_COLOR);

    let py = BY;
    draw_label(surf, PX, py,       "SCORE",        TEXT_COLOR);
    draw_value(surf, PX, py + 20,  game.score,     PIECE_COLORS[2]); // yellow
    draw_label(surf, PX, py + 60,  "LINES",        TEXT_COLOR);
    draw_value(surf, PX, py + 80,  game.lines,     PIECE_COLORS[2]);
    draw_label(surf, PX, py + 120, "LEVEL",        TEXT_COLOR);
    draw_value(surf, PX, py + 140, game.level,     PIECE_COLORS[2]);

    // Next piece preview
    draw_label(surf, PX, py + 180, "NEXT", TEXT_COLOR);
    let mini_x = PX;
    let mini_y = py + 204;
    for (x, y) in piece_cells(game.next_kind, 0, 0, 0) {
        if x >= 0 && y >= 0 {
            let px2 = mini_x + x as u32 * 14;
            let py2 = mini_y + y as u32 * 14;
            fill(surf, px2, py2, 13, 13, PIECE_COLORS[game.next_kind]);
        }
    }

    if game.over {
        let bw = COLS as u32 * CELL_PX;
        fill(surf, BX, BY + ROWS as u32 * CELL_PX / 2 - 20, bw, 44, BG_COLOR);
        draw_str2x(surf, BX + 4, BY + ROWS as u32 * CELL_PX / 2 - 8, b"GAME OVER", PIECE_COLORS[6]);
    }

    surf.damage_all();
}

// ── Primitives ────────────────────────────────────────────────────────────────

fn fill(surf: &mut ViSurface, x: u32, y: u32, w: u32, h: u32, bgra: u32) {
    let stride = surf.stride();
    let sw = surf.width(); let sh = surf.height();
    let pixels = surf.pixels_mut();
    let [b,g,r,a] = bgra.to_le_bytes();
    let x1 = (x + w).min(sw); let y1 = (y + h).min(sh);
    for py in y..y1 {
        for px in x..x1 {
            let o = py as usize * stride + px as usize * 4;
            pixels[o]=b; pixels[o+1]=g; pixels[o+2]=r; pixels[o+3]=a;
        }
    }
}

fn draw_cell(surf: &mut ViSurface, x: u32, y: u32, bgra: u32) {
    fill(surf, x, y, CELL_PX, CELL_PX, BORDER_COLOR);
    fill(surf, x+1, y+1, CELL_PX-2, CELL_PX-2, bgra);
}

fn draw_char(surf: &mut ViSurface, x: u32, y: u32, c: u8, bgra: u32, scale: u32) {
    let stride = surf.stride();
    let sw = surf.width() as i32; let sh = surf.height() as i32;
    let pixels = surf.pixels_mut();
    let [b,g,r,a] = bgra.to_le_bytes();
    let idx = if c >= 0x20 && c <= 0x7E { (c-0x20) as usize } else { 0 };
    for row in 0..8i32 {
        let mask = FONT8X8[idx][row as usize];
        for col in 0..8i32 {
            if mask & (0x80u8 >> col as u32) == 0 { continue; }
            for dy in 0..scale as i32 {
                for dx in 0..scale as i32 {
                    let qx = x as i32 + col*scale as i32 + dx;
                    let qy = y as i32 + row*scale as i32 + dy;
                    if qx<0||qx>=sw||qy<0||qy>=sh { continue; }
                    let o = qy as usize*stride + qx as usize*4;
                    pixels[o]=b; pixels[o+1]=g; pixels[o+2]=r; pixels[o+3]=a;
                }
            }
        }
    }
}

fn draw_str2x(surf: &mut ViSurface, x: u32, y: u32, s: &[u8], color: u32) {
    for (i, &c) in s.iter().enumerate() {
        draw_char(surf, x + i as u32 * 16, y, c, color, 2);
    }
}

fn draw_label(surf: &mut ViSurface, x: u32, y: u32, s: &str, color: u32) {
    draw_str2x(surf, x, y, s.as_bytes(), color);
}

fn draw_value(surf: &mut ViSurface, x: u32, y: u32, n: u32, color: u32) {
    // Format n into a small stack buffer (no alloc)
    let mut buf = [0u8; 10];
    let mut v = n; let mut len = 0usize;
    if v == 0 { buf[0] = b'0'; len = 1; }
    else {
        while v > 0 { buf[len] = b'0' + (v % 10) as u8; v /= 10; len += 1; }
        buf[..len].reverse();
    }
    draw_str2x(surf, x, y, &buf[..len], color);
}
