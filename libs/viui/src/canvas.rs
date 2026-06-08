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

    /// Draw text starting at `pos` (top-left). `style.size_px == 0` uses bitmap 8×8.
    fn draw_text(&mut self, pos: Point, text: &str, style: TextStyle);

    /// Draw scalable text at `pos` (top-left) using a GlyphAtlas.
    ///
    /// `px` is the font size in pixels. Default impl falls back to bitmap 8×8.
    fn draw_text_scaled(
        &mut self,
        pos: Point,
        text: &str,
        _px: f32,
        color: Color,
        _atlas: &mut ostd::font_atlas::GlyphAtlas,
    ) {
        self.draw_text(pos, text, TextStyle { color, size_px: 0 });
    }

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
    pixels:       &'fb mut [u8],
    stride:       u32,
    width:        u32,
    height:       u32,
    clip_stack:   [Rect; CLIP_STACK_DEPTH],
    clip_depth:   usize,
    // Integer (x0,y0,x1,y1) shadow of clip_stack — avoids f32 casts in put_pixel hot path.
    clip_stack_i: [(i32, i32, i32, i32); CLIP_STACK_DEPTH],
}

impl<'fb> FramebufferCanvas<'fb> {
    pub fn new(pixels: &'fb mut [u8], stride: u32, width: u32, height: u32) -> Self {
        let full = Rect { x: 0.0, y: 0.0, w: width as f32, h: height as f32 };
        let mut clip_stack = [Rect::default(); CLIP_STACK_DEPTH];
        clip_stack[0] = full;
        let mut clip_stack_i = [(0i32, 0i32, 0i32, 0i32); CLIP_STACK_DEPTH];
        clip_stack_i[0] = (0, 0, width as i32, height as i32);
        Self { pixels, stride, width, height, clip_stack, clip_depth: 0, clip_stack_i }
    }

    #[inline]
    fn active_clip(&self) -> Rect { self.clip_stack[self.clip_depth] }

    /// Write one pixel with src-over alpha blending.
    #[inline]
    fn put_pixel(&mut self, x: i32, y: i32, color: Color) {
        if x < 0 || y < 0 { return; }
        let (px, py) = (x as u32, y as u32);
        if px >= self.width || py >= self.height { return; }
        let (cx0, cy0, cx1, cy1) = self.clip_stack_i[self.clip_depth];
        if (px as i32) < cx0 || (px as i32) >= cx1 { return; }
        if (py as i32) < cy0 || (py as i32) >= cy1 { return; }
        let off = (py * self.stride + px * 4) as usize;
        if off + 3 >= self.pixels.len() { return; }
        // u32 load: LLVM emits single LDR on aligned targets
        let dst = Color(u32::from_le_bytes([
            self.pixels[off],
            self.pixels[off + 1],
            self.pixels[off + 2],
            self.pixels[off + 3],
        ]));
        let out = Color::blend_over(color, dst);
        // u32 store: LLVM emits single STR on aligned targets
        self.pixels[off..off + 4].copy_from_slice(&out.0.to_le_bytes());
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
            // Fast path: no blending — precompute bytes once, write as u32 per pixel
            let pixel = color.0.to_le_bytes();
            for y in y0..y1 {
                if y < 0 || y as u32 >= self.height { continue; }
                let row_off = (y as u32 * self.stride) as usize;
                for x in x0..x1 {
                    if x < 0 || x as u32 >= self.width { continue; }
                    let off = row_off + (x as usize) * 4;
                    if off + 3 < self.pixels.len() {
                        self.pixels[off..off + 4].copy_from_slice(&pixel);
                    }
                }
            }
        } else {
            // Alpha blend inline — coords are pre-clipped so put_pixel bounds checks are
            // redundant. Precompute channels once; compute row_off once per row.
            let sa  = color.a() as u32;
            let inv = 255 - sa;
            let cb  = color.b() as u32;
            let cg  = color.g() as u32;
            let cr  = color.r() as u32;
            for y in y0..y1 {
                let row_off = y as usize * self.stride as usize;
                for x in x0..x1 {
                    let off = row_off + x as usize * 4;
                    if off + 3 >= self.pixels.len() { continue; }
                    let db = self.pixels[off    ] as u32;
                    let dg = self.pixels[off + 1] as u32;
                    let dr = self.pixels[off + 2] as u32;
                    let out: u32 = ((cb * sa + db * inv) / 255)
                                 | (((cg * sa + dg * inv) / 255) << 8)
                                 | (((cr * sa + dr * inv) / 255) << 16)
                                 | (0xFF << 24);
                    self.pixels[off..off + 4].copy_from_slice(&out.to_le_bytes());
                }
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
        let w = self.width as i32;
        let h = self.height as i32;
        let opaque = style.color.a() == 255;
        // Precompute bytes once — avoids b()/g()/r() extraction inside glyph loop
        let pixel = style.color.0.to_le_bytes();
        let clip = self.active_clip();

        for ch in text.chars() {
            // A — skip char entirely outside screen bounds
            if cx >= w || cx + 8 <= 0 || cy >= h || cy + 8 <= 0 {
                cx += 8;
                continue;
            }

            // FONT8X8 covers 0x20..=0x7E (95 glyphs); index 0 = space (0x20)
            let code = ch as u32;
            let idx = if (0x20..=0x7E).contains(&code) { (code - 0x20) as usize } else { (b'?' - 0x20) as usize };
            let glyph = &FONT8X8[idx];

            // Fast path: char fully in screen bounds + clip + opaque — no per-pixel checks, no blend
            let fully_in_bounds = cx >= 0 && cx + 8 <= w && cy >= 0 && cy + 8 <= h;
            let fully_in_clip   = (cx as f32) >= clip.x
                && (cx + 8) as f32 <= clip.x + clip.w
                && (cy as f32) >= clip.y
                && (cy + 8) as f32 <= clip.y + clip.h;

            if opaque && fully_in_bounds && fully_in_clip {
                for (row, &bits) in glyph.iter().enumerate() {
                    // C — skip empty glyph rows (space, punctuation)
                    if bits == 0 { continue; }
                    // B — row byte offset computed once per row
                    let row_off = (cy as usize + row) * self.stride as usize;
                    for col in 0..8usize {
                        if bits & (0x80u8 >> col) != 0 {
                            let off = row_off + (cx as usize + col) * 4;
                            self.pixels[off..off + 4].copy_from_slice(&pixel);
                        }
                    }
                }
            } else {
                // Slow path — boundary chars, semi-transparent, or partial clip
                for row in 0..8i32 {
                    let bits = glyph[row as usize];
                    if bits == 0 { continue; }  // C — empty row skip even in slow path
                    for col in 0..8i32 {
                        if bits & (0x80u8 >> col) != 0 {
                            self.put_pixel(cx + col, cy + row, style.color);
                        }
                    }
                }
            }
            cx += 8;
        }
    }

    fn draw_text_scaled(
        &mut self,
        pos: Point,
        text: &str,
        px: f32,
        color: Color,
        atlas: &mut ostd::font_atlas::GlyphAtlas,
    ) {
        let baseline_y = pos.y + atlas.ascender(px);
        let mut draw_x = pos.x;

        for c in text.chars() {
            let (metrics, bitmap) = atlas.rasterize(c, px);
            if metrics.width == 0 {
                draw_x += metrics.advance_width;
                continue;
            }
            // Convert math y-up metrics to screen y-down position.
            // glyph top in screen coords = baseline_y - (ymin + height)
            let gx = draw_x + metrics.xmin as f32;
            let gy = baseline_y - (metrics.ymin as f32 + metrics.height as f32);

            for row in 0..metrics.height {
                for col in 0..metrics.width {
                    let coverage = bitmap[row * metrics.width + col];
                    if coverage == 0 { continue; }
                    let screen_x = (gx + col as f32) as i32;
                    let screen_y = (gy + row as f32) as i32;
                    // Modulate alpha by coverage
                    let alpha = ((color.a() as u32 * coverage as u32) / 255) as u8;
                    self.put_pixel(screen_x, screen_y, color.with_alpha(alpha));
                }
            }
            draw_x += metrics.advance_width;
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
            let dst_row_off = dst_y as usize * self.stride as usize;
            for dx in 0..(dst_x1 - dst_x0) {
                let sx = (dx_off + dx) as usize;
                let src_off = sy * src_stride as usize + sx * 4;
                if src_off + 3 >= pixels.len() { continue; }
                let dst_off = dst_row_off + (dst_x0 + dx) as usize * 4;
                if dst_off + 3 >= self.pixels.len() { continue; }
                let sa = pixels[src_off + 3];
                if sa == 0 { continue; }
                if sa == 255 {
                    // Opaque source: direct 4-byte copy, zero blend arithmetic.
                    self.pixels[dst_off..dst_off + 4]
                        .copy_from_slice(&pixels[src_off..src_off + 4]);
                } else {
                    // Partial alpha: inline blend over destination.
                    let sa32 = sa as u32;
                    let inv  = 255 - sa32;
                    let sb = pixels[src_off    ] as u32;
                    let sg = pixels[src_off + 1] as u32;
                    let sr = pixels[src_off + 2] as u32;
                    let db = self.pixels[dst_off    ] as u32;
                    let dg = self.pixels[dst_off + 1] as u32;
                    let dr = self.pixels[dst_off + 2] as u32;
                    let out: u32 = ((sb * sa32 + db * inv) / 255)
                                 | (((sg * sa32 + dg * inv) / 255) << 8)
                                 | (((sr * sa32 + dr * inv) / 255) << 16)
                                 | (0xFF << 24);
                    self.pixels[dst_off..dst_off + 4].copy_from_slice(&out.to_le_bytes());
                }
            }
        }
    }

    fn clip_push(&mut self, rect: Rect) {
        let current = self.active_clip();
        let new_clip = current.intersect(&rect).unwrap_or(Rect::ZERO);
        if self.clip_depth + 1 < CLIP_STACK_DEPTH {
            self.clip_depth += 1;
            self.clip_stack[self.clip_depth] = new_clip;
            self.clip_stack_i[self.clip_depth] = (
                new_clip.x as i32,
                new_clip.y as i32,
                (new_clip.x + new_clip.w) as i32,
                (new_clip.y + new_clip.h) as i32,
            );
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
