// SPDX-License-Identifier: MIT
//! Minimal tick-based app runner for ViUI v2.
//!
//! # Repaint strategy
//!
//! Two independent dirty flags:
//! - `layout_dirty`: set by consumed events or `mark_dirty()`. Triggers full
//!   layout + Signal re-subscribe (updates captured bounds) + full repaint.
//! - `dirty_region`: set by Signal subscriptions. Triggers partial repaint with
//!   the cached layout — no layout pass, O(m) paint where m = dirty widgets.
//!
//! Path A (structural): event consumed → layout → re-subscribe → mark_all
//! Path B (signal-only): signal fires → mark(widget_bounds) → partial repaint
//!
//! # Animation
//!
//! Add `Box<dyn Animatable>` via `add_animation()`. On every `tick_with_dt()`
//! call all active animations are advanced by `dt_ms` before event processing.
//! Animations that change a `Signal<f32>` trigger Path B automatically.
//!
//! # Usage
//!
//! ```rust,ignore
//! let count = Signal::new(0i32);
//! let label = Label::new(count.map(|n| format!("Count: {n}")));
//! let cnt2  = count.clone();
//! let btn   = Button::new("Increment", move || cnt2.update(|n| *n += 1));
//! let root  = vstack!(label, btn);
//!
//! let mut app = ViApp::new(Box::new(root), Box::new(renderer));
//! loop {
//!     app.tick(&collect_input_events());
//!     // or: app.tick_with_dt(&events, elapsed_ms);
//! }
//! ```

extern crate alloc;
use alloc::{boxed::Box, rc::Rc, vec::Vec};
use core::cell::RefCell;

use crate::animation::Animatable;
use crate::dirty::{DirtyRect, DirtyRegion};
use crate::event::Event;
use crate::font_context::FontContext;
use crate::layout::{Constraints, Size};
use crate::node::ViNode;
use crate::render_ctx::RenderCtx;
use crate::renderer::ViRenderer;
use crate::signal::SubscriptionHandle;

/// Tick-based app runner.
///
/// Call `tick()` or `tick_with_dt()` from the cell's main loop, passing any
/// buffered input events and — for animation — the milliseconds elapsed since
/// the last call.
pub struct ViApp {
    root:          Box<dyn ViNode>,
    renderer:      Box<dyn ViRenderer>,
    font_ctx:      FontContext,
    animations:    Vec<Box<dyn Animatable>>,
    dirty_region:  DirtyRegion,
    dirty_handles: Vec<SubscriptionHandle>,
    layout_dirty:  bool,
}

impl ViApp {
    /// Create a new app with no scalable font (8×8 bitmap fallback).
    ///
    /// The first tick always renders a full frame.
    pub fn new(root: Box<dyn ViNode>, renderer: Box<dyn ViRenderer>) -> Self {
        let dirty_region: DirtyRegion = Rc::new(RefCell::new(DirtyRect::new()));
        Self {
            root,
            renderer,
            font_ctx:     FontContext::no_font(),
            animations:   Vec::new(),
            dirty_region,
            dirty_handles: Vec::new(),
            layout_dirty:  true,
        }
    }

    /// Override the default 8×8 bitmap font with a scalable TTF font.
    ///
    /// `font_bytes`: raw TTF/OTF data (use `include_bytes!` for embedded fonts).
    /// `size_px`: default text height in pixels.
    /// Returns `self` unchanged when the font bytes are invalid.
    pub fn with_font(mut self, font_bytes: &[u8], size_px: f32) -> Self {
        if let Some(ctx) = FontContext::with_font(font_bytes, size_px) {
            self.font_ctx = ctx;
        }
        self
    }

    /// Register an animation that is advanced every `tick_with_dt()` call.
    ///
    /// Animations typically wrap an `AnimatedSignal<f32>` and drive a widget
    /// signal — no widget code changes are required.
    pub fn add_animation(&mut self, anim: Box<dyn Animatable>) {
        self.animations.push(anim);
    }

    /// Process events and render if dirty. Returns `true` if a frame was rendered.
    ///
    /// Equivalent to `tick_with_dt(events, 0)` — animations are not advanced.
    pub fn tick(&mut self, events: &[Event]) -> bool {
        self.tick_with_dt(events, 0)
    }

    /// Process events, advance animations by `dt_ms` milliseconds, and render
    /// if dirty. Returns `true` if a frame was rendered.
    ///
    /// `dt_ms`: milliseconds elapsed since the last call. Pass `0` to skip
    /// animation advancement (useful for initial render or event-only ticks).
    pub fn tick_with_dt(&mut self, events: &[Event], dt_ms: u32) -> bool {
        // ── Advance animations ────────────────────────────────────────────────
        // Animations update Signal<f32> values which trigger dirty-region marks
        // via their subscriptions — no explicit dirty flag needed here.
        if dt_ms > 0 {
            for anim in &mut self.animations {
                anim.tick(dt_ms);
            }
        }

        // ── Process input events ──────────────────────────────────────────────
        for e in events {
            if self.root.event(e) { self.layout_dirty = true; }
        }

        // ── Path A: structural change → full layout + re-subscribe ────────────
        if self.layout_dirty {
            self.layout_dirty = false;
            let (w, h) = self.renderer.size();
            self.root.layout(Constraints::root(Size::new(w as f32, h as f32)));
            self.dirty_handles = self.root.collect_dirty_handles(
                Rc::clone(&self.dirty_region)
            );
            self.dirty_region.borrow_mut().mark_all(w as f32, h as f32);
        }

        // ── Path B: render accumulated dirty region ───────────────────────────
        let damage = self.dirty_region.borrow_mut().take();
        if damage.is_none() { return false; }

        // Split borrows: renderer, font_ctx, and root are disjoint fields.
        let renderer  = &mut self.renderer;
        let font_ctx  = &mut self.font_ctx;
        let root      = &self.root;

        renderer.render(damage, &mut |canvas| {
            let mut cx = RenderCtx { canvas, font: font_ctx };
            root.paint(&mut cx);
        });
        true
    }

    /// Force a full repaint on the next `tick()` call.
    pub fn mark_dirty(&mut self) { self.layout_dirty = true; }
}
