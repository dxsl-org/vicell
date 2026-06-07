//! `ViCanvas` trait + `FramebufferCanvas` software rasterizer.
//!
//! Color layout: `Color(u32)` = BGRA packed LE: bits 0–7 = B, 8–15 = G, 16–23 = R, 24–31 = A.
//! This matches BGRA8888 little-endian framebuffer byte order (B at lowest address).

use crate::layout::{Point, Rect};

// ─── Color ───────────────────────────────────────────────────────────────────

/// Packed BGRA colour (matches VirtIO GPU wire format and compositor blending).
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub struct Color(pub u32);

impl Color {
    pub const TRANSPARENT: Self = Self(0x0000_0000);
    pub const BLACK:       Self = Self(0xFF00_0000);
    pub const WHITE:       Self = Self(0xFFFF_FFFF);
    pub const RED:         Self = Self(0xFF00_00FF);
    pub const GREEN:       Self = Self(0xFF00_FF00);
    pub const BLUE:        Self = Self(0xFFFF_0000);
    pub const GRAY:        Self = Self(0xFF80_8080);
    pub const DARK_GRAY:   Self = Self(0xFF40_4040);
    pub const YELLOW:      Self = Self(0xFF00_FFFF);  // BGRA: B=0 G=FF R=FF A=FF
    pub const CYAN:        Self = Self(0xFFFF_FF00);  // BGRA: B=FF G=FF R=0 A=FF
    pub const MAGENTA:     Self = Self(0xFFFF_00FF);  // BGRA: B=FF G=0 R=FF A=FF

    /// Construct from BGRA channels.
    pub const fn bgra(b: u8, g: u8, r: u8, a: u8) -> Self {
        Self((b as u32) | ((g as u32) << 8) | ((r as u32) << 16) | ((a as u32) << 24))
    }

    /// Construct from RGB channels (fully opaque).
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self { Self::bgra(b, g, r, 0xFF) }

    #[inline] pub fn b(self) -> u8 { (self.0        & 0xFF) as u8 }
    #[inline] pub fn g(self) -> u8 { (self.0 >>  8  & 0xFF) as u8 }
    #[inline] pub fn r(self) -> u8 { (self.0 >> 16  & 0xFF) as u8 }
    #[inline] pub fn a(self) -> u8 { (self.0 >> 24  & 0xFF) as u8 }

    pub fn with_alpha(self, a: u8) -> Self {
        Self((self.0 & 0x00FF_FFFF) | ((a as u32) << 24))
    }

    /// Alpha-blend `src` over `dst` (both BGRA). Uses `src.a` as blend factor.
    #[inline]
    pub fn blend_over(src: Color, dst: Color) -> Color {
        let sa = src.a() as u32;
        if sa == 0   { return dst; }
        if sa == 255 { return src; }
        let inv = 255 - sa;
        Color::bgra(
            ((src.b() as u32 * sa + dst.b() as u32 * inv) / 255) as u8,
            ((src.g() as u32 * sa + dst.g() as u32 * inv) / 255) as u8,
            ((src.r() as u32 * sa + dst.r() as u32 * inv) / 255) as u8,
            255,
        )
    }
}

// ─── TextStyle ───────────────────────────────────────────────────────────────

/// Text rendering parameters.
#[derive(Copy, Clone, Debug)]
pub struct TextStyle {
    pub color: Color,
    /// Font size in pixels. `0` = use bitmap 8×8 fallback font.
    pub size_px: u16,
}

impl Default for TextStyle {
    fn default() -> Self { Self { color: Color::WHITE, size_px: 0 } }
}

impl TextStyle {
    pub const DEFAULT: Self = Self { color: Color::WHITE, size_px: 0 };
}

// ─── ViCanvas ────────────────────────────────────────────────────────────────

/// Abstract pixel drawing surface.
///
/// All coordinates are screen-space (absolute pixels).
/// Clip stack: operations outside the current clip rect are silently discarded.
pub trait ViCanvas {
    /// Fill `rect` with a solid colour.
    fn fill_rect(&mut self, rect: Rect, color: Color);

    /// Draw a line from `a` to `b` (Bresenham integer algorithm in P02).
    fn draw_line(&mut self, a: Point, b: Point, color: Color);

    /// Draw text starting at `pos` (baseline). `style.size_px == 0` uses bitmap 8×8.
    fn draw_text(&mut self, pos: Point, text: &str, style: TextStyle);

    /// Blit raw BGRA pixels from `pixels` into `dest`.
    ///
    /// `src_stride`: bytes per row in the source buffer.
    fn draw_image(&mut self, dest: Rect, pixels: &[u8], src_stride: u32);

    /// Push a clip rectangle (intersects with any existing clip).
    fn clip_push(&mut self, rect: Rect);

    /// Pop the most recently pushed clip rectangle.
    fn clip_pop(&mut self);

    /// Current clip rectangle, or `None` if no clip is active.
    fn clip_rect(&self) -> Option<Rect>;

    /// Raw pixel buffer — used by `PaintCx::paint_text()` for glyph atlas blitting.
    fn pixels_mut(&mut self) -> &mut [u8];

    /// Bytes per row in the pixel buffer.
    fn stride(&self) -> u32;

    fn width(&self) -> u32;
    fn height(&self) -> u32;
}

// ─── FramebufferCanvas ───────────────────────────────────────────────────────

const CLIP_STACK_DEPTH: usize = 16;

/// Software rasterizer that writes directly into a BGRA8888 framebuffer slice.
///
/// `'fb` ties the canvas to the caller's borrow of the compositor surface buffer.
/// `stride` must be `>= width * 4`; mismatches cause silent pixel corruption.
pub struct FramebufferCanvas<'fb> {
    pixels:     &'fb mut [u8],
    stride:     u32,
    width:      u32,
    height:     u32,
    clip_stack: [Rect; CLIP_STACK_DEPTH],
    clip_depth: usize,
}

impl<'fb> FramebufferCanvas<'fb> {
    pub fn new(pixels: &'fb mut [u8], stride: u32, width: u32, height: u32) -> Self {
        let full = Rect { x: 0.0, y: 0.0, w: width as f32, h: height as f32 };
        let mut clip_stack = [Rect::default(); CLIP_STACK_DEPTH];
        clip_stack[0] = full;
        Self { pixels, stride, width, height, clip_stack, clip_depth: 0 }
    }

    #[inline]
    fn active_clip(&self) -> Rect { self.clip_stack[self.clip_depth] }

    /// Write one pixel with src-over alpha blending.
    #[inline]
    fn put_pixel(&mut self, x: i32, y: i32, color: Color) {
        if x < 0 || y < 0 { return; }
        let (px, py) = (x as u32, y as u32);
        if px >= self.width || py >= self.height { return; }
        let clip = self.clip_stack[self.clip_depth];
        if (px as f32) < clip.x || (px as f32) >= clip.x + clip.w { return; }
        if (py as f32) < clip.y || (py as f32) >= clip.y + clip.h { return; }
        let off = (py * self.stride + px * 4) as usize;
        if off + 3 >= self.pixels.len() { return; }
        let dst = Color::bgra(
            self.pixels[off],
            self.pixels[off + 1],
            self.pixels[off + 2],
            self.pixels[off + 3],
        );
        let out = Color::blend_over(color, dst);
        self.pixels[off]     = out.b();
        self.pixels[off + 1] = out.g();
        self.pixels[off + 2] = out.r();
        self.pixels[off + 3] = out.a();
    }
}

impl<'fb> ViCanvas for FramebufferCanvas<'fb> {
    fn fill_rect(&mut self, rect: Rect, color: Color) {
        let clip = self.active_clip();
        let clipped = match rect.intersect(&clip) { Some(r) => r, None => return };

        let x0 = clipped.x as i32;
        let y0 = clipped.y as i32;
        let x1 = (clipped.x + clipped.w) as i32;
        let y1 = (clipped.y + clipped.h) as i32;

        if color.a() == 255 {
            // Fast path: no blending
            for y in y0..y1 {
                if y < 0 || y as u32 >= self.height { continue; }
                let row_off = (y as u32 * self.stride) as usize;
                for x in x0..x1 {
                    if x < 0 || x as u32 >= self.width { continue; }
                    let off = row_off + (x as usize) * 4;
                    if off + 3 < self.pixels.len() {
                        self.pixels[off]     = color.b();
                        self.pixels[off + 1] = color.g();
                        self.pixels[off + 2] = color.r();
                        self.pixels[off + 3] = 0xFF;
                    }
                }
            }
        } else {
            for y in y0..y1 {
                for x in x0..x1 { self.put_pixel(x, y, color); }
            }
        }
    }

    fn draw_line(&mut self, a: Point, b: Point, color: Color) {
        // Bresenham integer line
        let mut x0 = a.x as i32;
        let mut y0 = a.y as i32;
        let x1 = b.x as i32;
        let y1 = b.y as i32;
        let dx =  (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1i32 } else { -1 };
        let sy = if y0 < y1 { 1i32 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.put_pixel(x0, y0, color);
            if x0 == x1 && y0 == y1 { break; }
            let e2 = 2 * err;
            if e2 >= dy { err += dy; x0 += sx; }
            if e2 <= dx { err += dx; y0 += sy; }
        }
    }

    fn draw_text(&mut self, pos: Point, text: &str, style: TextStyle) {
        // Reads FONT8X8 directly — ostd::font::draw_text uses a different byte-order
        // convention and cannot be used here without producing incorrect colours.
        use ostd::font::FONT8X8;
        let mut cx = pos.x as i32;
        let cy = pos.y as i32;
        for ch in text.chars() {
            // FONT8X8 covers 0x20..=0x7E (95 glyphs); index 0 = space (0x20)
            let code = ch as u32;
            let idx = if code >= 0x20 && code <= 0x7E { (code - 0x20) as usize } else { (b'?' - 0x20) as usize };
            let glyph = &FONT8X8[idx];
            for row in 0..8i32 {
                let bits = glyph[row as usize];
                for col in 0..8i32 {
                    if bits & (0x80u8 >> col) != 0 {
                        self.put_pixel(cx + col, cy + row, style.color);
                    }
                }
            }
            cx += 8;
        }
    }

    fn draw_image(&mut self, dest: Rect, pixels: &[u8], src_stride: u32) {
        let clip = self.active_clip();
        let clipped = match dest.intersect(&clip) { Some(r) => r, None => return };
        let dx_off = (clipped.x - dest.x) as i32;
        let dy_off = (clipped.y - dest.y) as i32;
        let dst_x0 = clipped.x as i32;
        let dst_y0 = clipped.y as i32;
        let dst_x1 = (clipped.x + clipped.w) as i32;
        let dst_y1 = (clipped.y + clipped.h) as i32;

        for dy in 0..(dst_y1 - dst_y0) {
            let sy = (dy_off + dy) as usize;
            let dst_y = dst_y0 + dy;
            if dst_y < 0 || dst_y as u32 >= self.height { continue; }
            for dx in 0..(dst_x1 - dst_x0) {
                let sx = (dx_off + dx) as usize;
                let src_off = sy * src_stride as usize + sx * 4;
                if src_off + 3 >= pixels.len() { continue; }
                let src = Color::bgra(
                    pixels[src_off],
                    pixels[src_off + 1],
                    pixels[src_off + 2],
                    pixels[src_off + 3],
                );
                self.put_pixel(dst_x0 + dx, dst_y, src);
            }
        }
    }

    fn clip_push(&mut self, rect: Rect) {
        let current = self.active_clip();
        let new_clip = current.intersect(&rect).unwrap_or(Rect::ZERO);
        if self.clip_depth + 1 < CLIP_STACK_DEPTH {
            self.clip_depth += 1;
            self.clip_stack[self.clip_depth] = new_clip;
        }
    }

    fn clip_pop(&mut self) {
        if self.clip_depth > 0 { self.clip_depth -= 1; }
    }

    fn clip_rect(&self) -> Option<Rect> { Some(self.active_clip()) }

    fn pixels_mut(&mut self) -> &mut [u8] { self.pixels }
    fn stride(&self)  -> u32 { self.stride  }
    fn width(&self)   -> u32 { self.width   }
    fn height(&self)  -> u32 { self.height  }
}
