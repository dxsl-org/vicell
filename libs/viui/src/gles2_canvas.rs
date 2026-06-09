// SPDX-License-Identifier: MIT
//! GLES2-backed `ViCanvas` — feature-gated skeleton for the G2 hardware GPU path.
//!
//! Activated with `features = ["gles2"]`. All draw methods are stubs until the
//! compositor EGL surface grant is wired. Widget code calls the same `ViCanvas`
//! methods regardless of backend — no widget changes when this is activated.
//!
//! # Architecture
//! `Gles2Canvas` records draw calls; `flush()` will submit them as batched GL
//! draw calls (quad VBO, glyph atlas texture). Until EGL is wired, `flush()` is a no-op.
//!
//! # Usage (future)
//! ```ignore
//! let gl = Gl::from_egl_context(egl_ctx);
//! let mut canvas = Gles2Canvas::new(&gl, 1280, 720);
//! root.paint(&mut canvas);
//! canvas.flush();   // submit batched vertex buffers
//! egl_swap_buffers();
//! ```

extern crate alloc;
use alloc::vec::Vec;

use crate::canvas::{Color, TextStyle, ViCanvas};
use crate::layout::{Point, Rect};

const CLIP_DEPTH: usize = 16;

// ─── Gl ──────────────────────────────────────────────────────────────────────

/// Opaque GLES2 context handle supplied by the compositor.
///
/// This is a placeholder type — populated with real EGL bindings when
/// `sys_egl_surface` IPC is wired (G2+ work, after compositor Grant surfaces).
pub struct Gl {
    // Placeholder; real fields added when EGL surface grant lands.
    _opaque: (),
}

impl Gl {
    /// Construct a stub context (feature-check compilation only).
    pub fn stub() -> Self { Self { _opaque: () } }
}

// ─── Gles2Canvas ─────────────────────────────────────────────────────────────

/// GLES2-backed canvas. Records draw calls for batched GPU submission.
///
/// All `ViCanvas` methods are no-ops in this skeleton. The full implementation
/// will batch colored quads into a VBO and upload glyphs to a 512×512 R8 texture.
pub struct Gles2Canvas<'a> {
    gl:         &'a Gl,
    width:      u32,
    height:     u32,
    clip_stack: [Rect; CLIP_DEPTH],
    clip_depth: usize,
    /// Scratch buffer returned by `pixels_mut()`.
    /// The v2 `ViNode` paint path never calls `pixels_mut()` — empty is safe.
    scratch:    Vec<u8>,
}

impl<'a> Gles2Canvas<'a> {
    /// Create a canvas bound to the given GLES2 context.
    pub fn new(gl: &'a Gl, width: u32, height: u32) -> Self {
        let full = Rect { x: 0.0, y: 0.0, w: width as f32, h: height as f32 };
        let mut clip_stack = [Rect::default(); CLIP_DEPTH];
        clip_stack[0] = full;
        Self { gl, width, height, clip_stack, clip_depth: 0, scratch: Vec::new() }
    }

    /// Submit all batched vertex buffers to the GPU (no-op until EGL is wired).
    pub fn flush(&mut self) {
        // G2: iterate quad batches, bind VBOs, issue glDrawArrays.
        let _ = self.gl;
    }

    fn active_clip(&self) -> Rect { self.clip_stack[self.clip_depth] }
}

impl ViCanvas for Gles2Canvas<'_> {
    fn fill_rect(&mut self, rect: Rect, color: Color) {
        // G2: push (rect, color) into quad batch; flush emits one batched draw call.
        let _ = (rect, color);
    }

    fn draw_line(&mut self, a: Point, b: Point, color: Color) {
        let _ = (a, b, color);
    }

    fn draw_text(&mut self, pos: Point, text: &str, style: TextStyle) {
        // G2: upload glyphs to atlas texture; emit textured quads.
        let _ = (pos, text, style);
    }

    fn draw_image(&mut self, dest: Rect, pixels: &[u8], src_stride: u32) {
        // G2: upload pixels as a 2D texture; emit textured quad.
        let _ = (dest, pixels, src_stride);
    }

    fn clip_push(&mut self, rect: Rect) {
        // G2: call glScissor with the intersected clip rect.
        if self.clip_depth + 1 < CLIP_DEPTH {
            self.clip_depth += 1;
            let intersected = self.active_clip().intersect(&rect)
                .unwrap_or(Rect::ZERO);
            self.clip_stack[self.clip_depth] = intersected;
        }
    }

    fn clip_pop(&mut self) {
        if self.clip_depth > 0 { self.clip_depth -= 1; }
    }

    fn clip_rect(&self) -> Option<Rect> {
        Some(self.clip_stack[self.clip_depth])
    }

    fn pixels_mut(&mut self) -> &mut [u8] { &mut self.scratch }

    fn stride(&self) -> u32 { self.width * 4 }

    fn width(&self)  -> u32 { self.width }

    fn height(&self) -> u32 { self.height }
}
