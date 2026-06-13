//! Software cursor sprite — compile-time 16×16 BGRA8888 arrow bitmap.
//!
//! The arrow points top-left. Each row is encoded as a `u16` bitmask
//! (MSB = leftmost pixel). Bit set = white fill pixel; neighbouring unset
//! bits that are outline candidates = black outline pixels.

/// Cursor sprite width and height in pixels.
pub const CURSOR_W: u32 = 16;
pub const CURSOR_H: u32 = 16;

/// Hotspot offset from the top-left corner of the sprite (the arrow tip).
#[inline(always)]
pub const fn hotspot() -> (i32, i32) {
    (0, 0)
}

/// Arrow-shaped 1-bit fill mask, MSB = leftmost pixel per row.
///
/// The arrow points toward (0,0) — top-left corner.
const ARROW_MASK: [u16; CURSOR_H as usize] = [
    0b1000_0000_0000_0000,  //  0  █
    0b1100_0000_0000_0000,  //  1  ██
    0b1010_0000_0000_0000,  //  2  █ █
    0b1001_0000_0000_0000,  //  3  █  █
    0b1000_1000_0000_0000,  //  4  █   █
    0b1000_0100_0000_0000,  //  5  █    █
    0b1000_0010_0000_0000,  //  6  █     █
    0b1000_0001_0000_0000,  //  7  █      █
    0b1000_0000_1000_0000,  //  8  █       █
    0b1000_0000_0100_0000,  //  9  █        █
    0b1000_0000_1110_0000,  // 10  █       ███
    0b1001_0100_0000_0000,  // 11  █  █ █
    0b1010_0010_0000_0000,  // 12  █ █   █
    0b1100_0001_0000_0000,  // 13  ██      █
    0b1000_0000_1000_0000,  // 14  █       █
    0b0000_0000_1110_0000,  // 15          ███
];

/// Black outline mask — pixels that border the arrow fill.
/// Derived from the fill mask: pixels adjacent to a set fill bit, but
/// not themselves set. Pre-computed to avoid per-pixel expansion at runtime.
const OUTLINE_MASK: [u16; CURSOR_H as usize] = [
    0b0111_0000_0000_0000,  //  0
    0b0010_1000_0000_0000,  //  1
    0b0101_0100_0000_0000,  //  2
    0b0100_1010_0000_0000,  //  3
    0b0100_0101_0000_0000,  //  4
    0b0100_0010_1000_0000,  //  5
    0b0100_0001_0100_0000,  //  6
    0b0100_0000_1010_0000,  //  7
    0b0100_0000_0101_0000,  //  8
    0b0100_0000_0010_1000,  //  9
    0b0100_0000_0001_0000,  // 10
    0b0100_1010_1100_0000,  // 11
    0b0101_0001_0100_0000,  // 12
    0b0010_1000_1010_0000,  // 13
    0b0100_0000_0101_0000,  // 14
    0b0000_0000_0001_0000,  // 15
];

/// White cursor fill pixel (BGRA8888).
const WHITE: [u8; 4] = [255, 255, 255, 255];

/// Black outline pixel — slightly transparent for a softer edge (BGRA8888).
const BLACK: [u8; 4] = [0, 0, 0, 220];

/// Return the BGRA8888 pixel for the cursor sprite at `(col, row)`,
/// or `None` for fully transparent (background pass-through).
///
/// Precondition: `row < CURSOR_H`, `col < CURSOR_W`.
#[inline]
pub fn cursor_pixel(row: u32, col: u32) -> Option<[u8; 4]> {
    if row >= CURSOR_H || col >= CURSOR_W {
        return None;
    }
    // MSB (bit 15) = leftmost pixel (col 0); LSB (bit 0) = rightmost (col 15).
    // col < CURSOR_W = 16, so the shift amount is 0..=15 (no overflow).
    let bit = 0x8000u16 >> (col as u16);
    if ARROW_MASK[row as usize] & bit != 0 {
        return Some(WHITE);
    }
    if OUTLINE_MASK[row as usize] & bit != 0 {
        return Some(BLACK);
    }
    None
}
