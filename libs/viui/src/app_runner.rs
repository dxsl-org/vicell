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
use crate::event::{Event, KeyCode, Modifiers};
use crate::font_context::FontContext;
use crate::layout::{Constraints, Size};
use crate::node::ViNode;
use crate::render_ctx::RenderCtx;
use crate::renderer::ViRenderer;
use crate::signal::SubscriptionHandle;
use crate::theme::{DarkTheme, ViTheme};

// ─── Key repeat ──────────────────────────────────────────────────────────────

/// Delay before the first software repeat fires (milliseconds).
const KEY_REPEAT_DELAY_MS: u64 = 500;

/// Interval between subsequent software repeat events (milliseconds).
const KEY_REPEAT_INTERVAL_MS: u64 = 50;

/// Tracks which key is currently held for software key-repeat injection.
///
/// # Hardware vs software repeat
/// The input service already sends `state=Repeated` events for hardware
/// auto-repeat (see `api::input::KeyState::Repeated`). `input_bridge::parse_input_message`
/// treats those identically to `Pressed`, so hardware repeat already works without
/// this struct. `KeyRepeatState` provides *software* repeat for platforms that do NOT
/// forward hardware repeat (serial consoles, touch keyboards, synthetic input in tests).
/// When hardware repeat arrives it naturally resets `held_ms` via the Pressed arm, so
/// the two paths do not fight.
struct KeyRepeatState {
    /// The key currently held, or `None` when no key is pressed.
    held_key:  Option<KeyCode>,
    /// Modifier state captured at the moment the key was pressed.
    held_mods: Modifiers,
    /// Accumulated milliseconds since the key was first pressed (reset on new press).
    held_ms:   u64,
    /// Accumulated milliseconds since the last repeat event fired (reset on each fire).
    since_last_ms: u64,
}

impl KeyRepeatState {
    const fn new() -> Self {
        Self {
            held_key:      None,
            held_mods:     Modifiers { shift: false, ctrl: false, alt: false },
            held_ms:       0,
            since_last_ms: 0,
        }
    }
}

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
    /// Active design-token set. Widgets read colors/spacing from `cx.theme`.
    theme:         Box<dyn ViTheme>,
    /// Software key-repeat state. See `KeyRepeatState` for rationale.
    key_repeat:    KeyRepeatState,
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
            font_ctx:      FontContext::no_font(),
            animations:    Vec::new(),
            dirty_region,
            dirty_handles: Vec::new(),
            layout_dirty:  true,
            theme:         Box::new(DarkTheme),
            key_repeat:    KeyRepeatState::new(),
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

    /// Switch the active theme. The next frame will render with the new tokens.
    pub fn set_theme<T: ViTheme + 'static>(&mut self, theme: T) {
        self.theme = Box::new(theme);
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
            // Track held key for software repeat injection (see KeyRepeatState).
            match e {
                Event::KeyPress { key, modifiers } => {
                    self.key_repeat.held_key      = Some(*key);
                    self.key_repeat.held_mods     = *modifiers;
                    self.key_repeat.held_ms       = 0;
                    self.key_repeat.since_last_ms = 0;
                }
                Event::KeyRelease { .. } => {
                    self.key_repeat.held_key = None;
                }
                _ => {}
            }
            if self.root.event(e) { self.layout_dirty = true; }
        }

        // ── Software key-repeat injection ─────────────────────────────────────
        // Only runs when time is advancing (dt_ms > 0) and a key is held.
        // Platforms that send hardware repeat events (api::input::KeyState::Repeated)
        // already inject repeats via input_bridge, so this path fires only when
        // held_ms crosses the thresholds without hardware-repeat interruption.
        if dt_ms > 0 {
            if let Some(key) = self.key_repeat.held_key {
                self.key_repeat.held_ms       += dt_ms as u64;
                self.key_repeat.since_last_ms += dt_ms as u64;

                if self.key_repeat.held_ms >= KEY_REPEAT_DELAY_MS
                    && self.key_repeat.since_last_ms >= KEY_REPEAT_INTERVAL_MS
                {
                    self.key_repeat.since_last_ms = 0;
                    let repeat_event = Event::KeyPress {
                        key,
                        modifiers: self.key_repeat.held_mods,
                    };
                    if self.root.event(&repeat_event) {
                        self.layout_dirty = true;
                    }
                }
            }
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

        // Split borrows: renderer, font_ctx, theme, and root are disjoint fields.
        let renderer  = &mut self.renderer;
        let font_ctx  = &mut self.font_ctx;
        let theme     = &*self.theme;
        let root      = &self.root;

        renderer.render(damage, &mut |canvas| {
            let mut cx = RenderCtx { canvas, font: font_ctx, theme };
            root.paint(&mut cx);
        });
        true
    }

    /// Force a full repaint on the next `tick()` call.
    pub fn mark_dirty(&mut self) { self.layout_dirty = true; }
}
