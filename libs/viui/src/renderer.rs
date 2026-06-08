// SPDX-License-Identifier: MIT
//! ViRenderer trait — abstract rendering backend for ViUI v2.
//!
//! # Backend selection
//!
//! G1: `FramebufferRenderer` (CPU software rasterizer, default)
//! G2+: GPU backend implementing the same trait; widget code is unchanged.
//!
//! # Object safety
//!
//! `ViRenderer` is object-safe; use `Box<dyn ViRenderer>` to store a
//! heap-allocated renderer when the concrete type is unknown at compile time.
//!
//! # Lifetime note
//!
//! `FramebufferCanvas<'fb>` borrows pixels from `ViSurface`. The closure pattern
//! in `render()` confines that borrow to the stack frame, avoiding any
//! self-referential struct or lifetime gymnastics.

use crate::canvas::{FramebufferCanvas, ViCanvas};
use crate::layout::Rect;
use ostd::display::ViSurface;

// ─── ViRenderer ──────────────────────────────────────────────────────────────

/// Abstract rendering backend.
///
/// # Contract
///
/// - Call `render()` once per frame, after collecting dirty rects via `DirtyRect`.
/// - All painting must happen inside the `draw` closure — canvas is invalid after return.
/// - `damage` is advisory: G1 ignores it (always damage_all); G2+ uses it for partial flip.
pub trait ViRenderer {
    /// Run a paint closure with exclusive canvas access, then submit the frame.
    ///
    /// `damage`: the screen region that changed this frame. `None` = full surface.
    fn render(&mut self, damage: Option<Rect>, draw: &mut dyn FnMut(&mut dyn ViCanvas));

    /// Surface dimensions in pixels as `(width, height)`.
    fn size(&self) -> (u32, u32);
}

// ─── FramebufferRenderer ─────────────────────────────────────────────────────

/// G1 CPU renderer wrapping a `ViSurface` + `FramebufferCanvas`.
///
/// The `FramebufferCanvas<'fb>` borrow is confined to the `render()` call
/// stack frame — no heap allocation, no unsafe, no self-referential struct.
pub struct FramebufferRenderer {
    surf: ViSurface,
}

impl FramebufferRenderer {
    pub fn new(surf: ViSurface) -> Self { Self { surf } }

    /// Unwrap the inner `ViSurface` (e.g. for IPC cleanup after app exit).
    pub fn into_surf(self) -> ViSurface { self.surf }
}

impl ViRenderer for FramebufferRenderer {
    fn render(&mut self, damage: Option<Rect>, draw: &mut dyn FnMut(&mut dyn ViCanvas)) {
        let stride = self.surf.stride() as u32;
        let (w, h) = (self.surf.width(), self.surf.height());
        let pixels = self.surf.pixels_mut();
        let mut canvas = FramebufferCanvas::new(pixels, stride, w, h);
        draw(&mut canvas);
        // G1: damage_all regardless of dirty rect; G2+ will use partial-flip.
        let _ = damage;
        self.surf.damage_all();
    }

    fn size(&self) -> (u32, u32) { (self.surf.width(), self.surf.height()) }
}
