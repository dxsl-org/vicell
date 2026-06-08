// SPDX-License-Identifier: MIT
//! `FontContext` — optional scalable font state for ViUI v2.
//!
//! Wraps an optional `GlyphAtlas` (fontdue-backed) with a default text size.
//! When no font is loaded, all text falls back to the 8×8 bitmap font in
//! `FramebufferCanvas::draw_text()`.
//!
//! # Usage
//!
//! ```rust,ignore
//! // No font — bitmap fallback everywhere
//! let ctx = FontContext::no_font();
//!
//! // Load a TTF font at default size 16px
//! let ctx = FontContext::with_font(include_bytes!("../assets/font.ttf"), 16.0)
//!               .expect("invalid font bytes");
//! ```

use ostd::font_atlas::GlyphAtlas;

/// Scalable font state shared across a full frame's paint pass.
///
/// Passed through `RenderCtx` to every `ViNode::paint()` call.
/// Widgets that do not need scalable text ignore the `atlas` field entirely.
pub struct FontContext {
    /// Cached glyph rasterizer. `None` = bitmap 8×8 fallback.
    pub atlas:   Option<GlyphAtlas>,
    /// Default font size in pixels used by text widgets.
    pub size_px: f32,
}

impl FontContext {
    /// No scalable font — all text widgets use the 8×8 bitmap fallback.
    pub fn no_font() -> Self {
        Self { atlas: None, size_px: 16.0 }
    }

    /// Load a TrueType / OpenType font from raw bytes.
    ///
    /// Returns `None` when the font data is invalid.
    pub fn with_font(font_bytes: &[u8], size_px: f32) -> Option<Self> {
        GlyphAtlas::new(font_bytes).map(|atlas| Self { atlas: Some(atlas), size_px })
    }

    /// Convenience: `with_font()` with a fallback to `no_font()`.
    ///
    /// Use when the font is optional — degraded but non-panicking.
    pub fn try_font(font_bytes: &[u8], size_px: f32) -> Self {
        Self::with_font(font_bytes, size_px).unwrap_or_else(FontContext::no_font)
    }

    /// True when a scalable atlas is loaded.
    pub fn has_font(&self) -> bool { self.atlas.is_some() }
}
