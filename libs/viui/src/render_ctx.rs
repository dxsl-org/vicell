// SPDX-License-Identifier: MIT
//! `RenderCtx` — combined canvas + font context + theme for `ViNode::paint()`.
//!
//! Bundles a mutable canvas reference with the frame's `FontContext` and
//! active `ViTheme` so every widget has access to drawing primitives, scalable
//! glyphs, and design tokens through a single argument.
//!
//! # Why a struct instead of two arguments?
//!
//! `ViNode::paint()` is an object-safe trait method. Rust requires object-safe
//! methods to have no more than one unsized parameter (the receiver). Passing
//! two `&mut dyn …` arguments is fine syntactically but introducing `RenderCtx`
//! is cleaner, extensible, and avoids ambiguity at call sites.

use crate::canvas::{Color, TextStyle, ViCanvas};
use crate::font_context::FontContext;
use crate::layout::Point;
use crate::theme::ViTheme;

/// Combined draw surface + font state + active theme for one paint pass.
///
/// Passed by mutable reference through the entire widget tree during paint.
/// Containers forward `cx` directly to children — no intermediate clone or reborrow.
pub struct RenderCtx<'a> {
    /// The pixel drawing surface for this frame.
    pub canvas: &'a mut dyn ViCanvas,
    /// Scalable glyph state. `atlas` may be `None` (bitmap fallback).
    pub font:   &'a mut FontContext,
    /// Active design token set. Widgets read colors and spacing from here
    /// rather than hardcoding values.
    pub theme:  &'a dyn ViTheme,
}

impl<'a> RenderCtx<'a> {
    /// Draw text, using the scalable atlas when available, otherwise 8×8 bitmap.
    ///
    /// Prefer this helper over calling `canvas.draw_text` directly so the
    /// fallback logic is in one place.
    #[inline]
    pub fn draw_text(&mut self, pos: Point, text: &str, color: Color) {
        match self.font.atlas.as_mut() {
            Some(atlas) => {
                let px = self.font.size_px;
                self.canvas.draw_text_scaled(pos, text, px, color, atlas);
            }
            None => {
                self.canvas.draw_text(
                    pos, text,
                    TextStyle { color, size_px: 0 },
                );
            }
        }
    }

    /// Draw text at an explicit pixel size, with atlas or bitmap fallback.
    #[inline]
    pub fn draw_text_at_size(&mut self, pos: Point, text: &str, color: Color, size_px: f32) {
        match self.font.atlas.as_mut() {
            Some(atlas) => self.canvas.draw_text_scaled(pos, text, size_px, color, atlas),
            None        => self.canvas.draw_text(pos, text, TextStyle { color, size_px: 0 }),
        }
    }

    /// Estimated character width at the current font size. Used for layout.
    ///
    /// Returns an approximation — exact metrics require a full layout pass with
    /// the atlas. Widgets that need tight text layout should call
    /// `self.font.atlas.as_mut().map(|a| a.advance(ch, px))` directly.
    #[inline]
    pub fn char_width(&self) -> f32 {
        if self.font.has_font() { self.font.size_px * 0.6 } else { 8.0 }
    }

    /// Line height at the current font size.
    #[inline]
    pub fn line_height(&self) -> f32 {
        if self.font.has_font() { self.font.size_px * 1.2 } else { 8.0 }
    }

    /// Re-borrow as a shorter-lived `RenderCtx`. Allows passing `cx` to child
    /// widgets when the parent still needs access afterward (split borrow).
    #[inline]
    pub fn reborrow(&mut self) -> RenderCtx<'_> {
        RenderCtx { canvas: self.canvas, font: self.font, theme: self.theme }
    }
}
