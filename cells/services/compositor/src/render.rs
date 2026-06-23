//! Software rasterizer — blends damaged surfaces into the screen framebuffer
//! and flushes the dirty region to the VirtIO GPU via the `GpuFlush` syscall.

extern crate alloc;

use alloc::vec;
use api::display::Rect;
use ostd::syscall::{sys_gpu_flush, sys_get_resolution};
use crate::cursor_sprite::{CURSOR_H, CURSOR_W, cursor_pixel};
use crate::surface_table::SurfaceTable;
use crate::z_order::ZOrder;

/// Screen framebuffer owned by the compositor (BGRA8888).
pub struct ScreenFb {
    pixels:  alloc::vec::Vec<u8>,
    /// Reusable staging buffer for GPU flush — pre-allocated to avoid per-frame heap churn.
    staging: alloc::vec::Vec<u8>,
    pub width:  u32,
    pub height: u32,
}

impl ScreenFb {
    /// Allocate a zeroed framebuffer of the given dimensions.
    pub fn new(width: u32, height: u32) -> Self {
        assert!(width > 0 && height > 0 && width <= 4096 && height <= 4096,
            "ScreenFb dimensions out of range: {}x{}", width, height);
        let full = (width * height * 4) as usize;
        Self {
            pixels:  vec![0u8; full],
            staging: vec![0u8; full],
            width,
            height,
        }
    }

    /// Blit one surface's pixels into the screen FB at its screen position.
    ///
    /// Clips to the screen boundary; surfaces that are partially off-screen
    /// are rendered up to the edge.
    fn blit_surface(&mut self, s: &crate::surface_table::SurfaceState) {
        let sx = s.x.max(0) as u32;
        let sy = s.y.max(0) as u32;
        let clip_x = (-s.x).max(0) as u32; // surface offset if partially off-screen
        let clip_y = (-s.y).max(0) as u32;
        let w = (s.w.saturating_sub(clip_x)).min(self.width.saturating_sub(sx));
        let h = (s.h.saturating_sub(clip_y)).min(self.height.saturating_sub(sy));
        if w == 0 || h == 0 { return; }

        let screen_stride = self.width as usize * 4;
        let surf_stride   = s.w as usize * 4;
        let surf_pixels   = s.pixels();

        for row in 0..h as usize {
            let dst_off = (sy as usize + row) * screen_stride + sx as usize * 4;
            let src_off = (clip_y as usize + row) * surf_stride + clip_x as usize * 4;
            let n = w as usize * 4;
            if dst_off + n <= self.pixels.len() && src_off + n <= surf_pixels.len() {
                self.pixels[dst_off..dst_off + n]
                    .copy_from_slice(&surf_pixels[src_off..src_off + n]);
            }
        }
    }

    /// Alpha-blend the 16×16 cursor sprite at `(cx, cy)` into the FB.
    ///
    /// Only pixels that fall inside `dirty` are written — avoids painting outside
    /// the region that will be flushed to the GPU this frame.
    /// Clips the sprite to screen bounds so a cursor near an edge is safe.
    ///
    /// Pixel format is BGRA8888 (native VirtIO GPU order). Alpha blend formula:
    ///   out = src * alpha/255 + dst * (255 - alpha)/255
    fn composite_cursor(&mut self, cx: i32, cy: i32, dirty: Rect) {
        let stride = self.width as usize * 4;
        for row in 0..CURSOR_H {
            let sy = cy + row as i32;
            if sy < 0 || sy >= self.height as i32 { continue; }
            for col in 0..CURSOR_W {
                let sx = cx + col as i32;
                if sx < 0 || sx >= self.width as i32 { continue; }
                // Skip pixels outside the dirty region (won't be flushed).
                if sx < dirty.x || sx >= dirty.x + dirty.w as i32
                    || sy < dirty.y || sy >= dirty.y + dirty.h as i32
                {
                    continue;
                }
                let Some(src) = cursor_pixel(row, col) else { continue };
                let alpha = src[3] as u32;
                if alpha == 0 { continue; }
                let dst_off = sy as usize * stride + sx as usize * 4;
                if dst_off + 4 > self.pixels.len() { continue; }
                // Straight-alpha blend over destination (BGRA channel order).
                for ch in 0..3usize {
                    let d = self.pixels[dst_off + ch] as u32;
                    let s = src[ch] as u32;
                    self.pixels[dst_off + ch] =
                        ((s * alpha + d * (255 - alpha)) / 255) as u8;
                }
                self.pixels[dst_off + 3] = 255; // opaque result
            }
        }
    }

    /// Flush `dirty_rect` from the screen FB to the GPU.
    ///
    /// Copies the dirty region into the pre-allocated staging buffer (no heap allocation)
    /// then hands the sub-slice to the kernel. Clamps to screen boundary.
    fn flush_rect(&mut self, dirty: Rect) {
        let x = dirty.x.max(0) as u32;
        let y = dirty.y.max(0) as u32;
        let w = dirty.w.min(self.width.saturating_sub(x));
        let h = dirty.h.min(self.height.saturating_sub(y));
        if w == 0 || h == 0 { return; }

        let stride  = self.width as usize * 4;
        let sub_len = (w * h * 4) as usize;
        for row in 0..h as usize {
            let src = (y as usize + row) * stride + x as usize * 4;
            let dst = row * w as usize * 4;
            let n   = w as usize * 4;
            if src + n <= self.pixels.len() && dst + n <= self.staging.len() {
                self.staging[dst..dst + n].copy_from_slice(&self.pixels[src..src + n]);
            }
        }
        let _ = sys_gpu_flush(&self.staging[..sub_len], x, y, w, h);
    }
}

/// Render one frame: blit all damaged surfaces, composite the cursor, then
/// flush the combined dirty rect to the VirtIO GPU.
///
/// `extra_dirty` is a compositor-initiated repaint region (cursor move, surface
/// destroyed/raised) that is unioned with per-surface damage before blitting.
/// `cursor_x/cursor_y` is the current logical mouse position used to draw the
/// 16×16 software cursor sprite on top of all surfaces.
///
/// `extra_dirty` is a compositor-initiated repaint region (e.g. surface just
/// destroyed or raised) that is unioned with per-surface damage before blitting.
///
/// Returns the dirty rect that was flushed, or `None` if nothing was dirty.
/// `fb` requires `&mut` because `flush_rect` writes into the staging buffer.
pub fn render_frame(
    fb: &mut ScreenFb,
    table: &mut SurfaceTable,
    z_order: &ZOrder,
    extra_dirty: Option<Rect>,
    cursor_x: i32,
    cursor_y: i32,
) -> Option<Rect> {
    // Seed with compositor-initiated dirty region (cursor move, surface destroyed/raised).
    let mut dirty: Option<Rect> = extra_dirty;
    for cap in z_order.iter_bottom_to_top() {
        if let Some(s) = table.get(cap) {
            if let Some(dmg) = s.damage {
                // Translate surface-local damage to screen coordinates.
                let screen_dmg = Rect {
                    x: s.x + dmg.x,
                    y: s.y + dmg.y,
                    w: dmg.w,
                    h: dmg.h,
                };
                dirty = Some(match dirty {
                    Some(acc) => acc.union(&screen_dmg),
                    None => screen_dmg,
                });
            }
        }
    }

    let dirty = dirty?;

    // Re-blit all surfaces that overlap the dirty rect (bottom to top).
    for cap in z_order.iter_bottom_to_top() {
        if let Some(s) = table.get(cap) {
            if s.screen_rect().intersects(&dirty) {
                fb.blit_surface(s);
            }
        }
    }

    // Clear damage on all surfaces.
    for cap in z_order.iter_bottom_to_top() {
        if let Some(s) = table.get_mut(cap) {
            s.clear_damage();
        }
    }

    // Composite the cursor sprite on top of all surfaces within the dirty rect.
    // The cursor is always drawn last so it occludes surface pixels.
    fb.composite_cursor(cursor_x, cursor_y, dirty);

    // Flush the dirty rect to the GPU.
    fb.flush_rect(dirty);
    Some(dirty)
}

/// Return the screen dimensions from the GPU's actual scanout resolution.
///
/// Calls `GpuGetResolution` syscall so all callers (ScreenFb alloc, GET_SCREEN_SIZE
/// replies, etc.) always agree with the hardware — no hardcoded fallback constants.
pub fn default_screen_size() -> (u32, u32) {
    sys_get_resolution()
}
