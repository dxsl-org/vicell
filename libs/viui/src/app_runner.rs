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
use alloc::{boxed::Box, collections::VecDeque, rc::Rc, vec::Vec};
use core::cell::RefCell;

use crate::animation::Animatable;
use crate::dirty::{DirtyRect, DirtyRegion};
use crate::event::{Event, KeyCode, Modifiers};
use crate::font_context::FontContext;
use crate::layout::{Constraints, Size};
use crate::node::ViNode;
use crate::overlay::{new_action_queue, OverlayAction, OverlayActionQueue, OverlayEntry};
use crate::render_ctx::RenderCtx;
use crate::renderer::ViRenderer;
use crate::signal::SubscriptionHandle;
use crate::theme::{DarkTheme, ViTheme};

// ─── Focus state ─────────────────────────────────────────────────────────────

/// Tracks Tab-order focus across all focusable widgets in the tree.
///
/// `list` is rebuilt after every layout pass (bounds are stale until then).
/// `idx` is clamped or cleared whenever the list shrinks.
struct FocusState {
    /// Screen rects of all focusable widgets, in tree (tab) order.
    list: alloc::vec::Vec<crate::layout::Rect>,
    /// Currently focused index into `list`, or `None` when no widget is focused.
    idx:  Option<usize>,
}

impl FocusState {
    fn new() -> Self { Self { list: alloc::vec::Vec::new(), idx: None } }

    /// Advance or reverse focus by one step, wrapping around.
    fn advance(&mut self, reverse: bool) {
        if self.list.is_empty() { self.idx = None; return; }
        self.idx = Some(match self.idx {
            None => if reverse { self.list.len() - 1 } else { 0 },
            Some(i) => if reverse {
                if i == 0 { self.list.len() - 1 } else { i - 1 }
            } else {
                (i + 1) % self.list.len()
            },
        });
    }

    /// Returns the screen rect of the currently focused widget, if any.
    fn current_bounds(&self) -> Option<crate::layout::Rect> {
        self.idx.and_then(|i| self.list.get(i).copied())
    }
}

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
    /// Tab-order focus state. Rebuilt after every layout pass.
    focus:         FocusState,
    /// Last known mouse position, updated on every MouseMove.
    /// Injected into MousePress/MouseRelease events whose pos is (0,0) because
    /// the input service encodes button events without position (caller must track).
    mouse_pos:     crate::layout::Point,
    /// Stack of overlay widgets (dialogs, drop-down popups). Rendered and
    /// dispatched on top of `root`; topmost entry wins events first.
    overlays:      Vec<OverlayEntry>,
    /// Deferred overlay mutations queued by widgets during `event()`.
    /// Drained once per frame after all input events are processed.
    action_queue:  OverlayActionQueue,
    /// Active toast notifications, in chronological order.
    toast_entries: VecDeque<crate::node_widgets::toast::ToastEntry>,
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
            focus:         FocusState::new(),
            mouse_pos:     crate::layout::Point::new(0.0, 0.0),
            overlays:      Vec::new(),
            action_queue:  new_action_queue(),
            toast_entries: VecDeque::new(),
        }
    }

    // ── Overlay API ──────────────────────────────────────────────────────────

    /// Return a clone of the action queue.
    ///
    /// Pass this clone to `Dialog` / `DropDown` constructors so they can push
    /// `OverlayAction` items without holding a borrow on `ViApp`.
    pub fn action_queue(&self) -> OverlayActionQueue {
        Rc::clone(&self.action_queue)
    }

    /// Push a widget as a blocking or non-blocking overlay.
    ///
    /// `blocking = true` — modal: all input is consumed by the overlay, the
    /// root does not see events while it is on screen (e.g. Dialog).
    /// `blocking = false` — non-modal: input passes through when not consumed
    /// by the overlay (e.g. DropDown popup).
    pub fn push_overlay(&mut self, widget: Box<dyn ViNode>, blocking: bool) {
        self.overlays.push(OverlayEntry {
            widget,
            blocking,
            dismiss_outside: !blocking,
            anchor_bounds:   None,
        });
        self.layout_dirty = true;
    }

    /// Pop the topmost overlay. No-op when the overlay stack is empty.
    pub fn pop_overlay(&mut self) {
        self.overlays.pop();
        self.layout_dirty = true;
    }

    /// Show a transient toast notification.
    ///
    /// Toasts are rendered above all content and auto-dismiss after
    /// `config.duration_ms` milliseconds (`0` = never auto-dismiss).
    pub fn show_toast(&mut self, config: crate::node_widgets::toast::ToastConfig) {
        let widget = crate::node_widgets::toast::Toast::new(&config);
        self.toast_entries.push_back(crate::node_widgets::toast::ToastEntry {
            config,
            elapsed_ms: 0,
            widget,
        });
        self.layout_dirty = true;
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

        // ── Drain action queue from previous frame ────────────────────────────
        // Widgets may have pushed actions during the last tick's event pass.
        // Drain before the new event pass so actions are visible immediately.
        self.drain_action_queue();

        // ── Process input events ──────────────────────────────────────────────
        for e in events {
            // Tab focus cycling — consumed by the focus system, not dispatched to root.
            if let Event::KeyPress { key: KeyCode::Tab, modifiers } = e {
                self.focus.advance(modifiers.shift);
                self.layout_dirty = true;
                continue;
            }

            // Enter activation — route directly to the focused widget via activate_at().
            // When a blocking overlay is active, activate_at on the overlay is more
            // appropriate; skip root activation in that case.
            if let Event::KeyPress { key: KeyCode::Enter, .. } = e {
                let top_blocking = self.overlays.last().map(|o| o.blocking).unwrap_or(false);
                if !top_blocking {
                    if let Some(b) = self.focus.current_bounds() {
                        if self.root.activate_at(b) { self.layout_dirty = true; }
                    }
                }
                continue;
            }

            // Track mouse position for button-event injection.
            // parse_mouse_button in input_bridge sets pos=(0,0) because the input
            // service encodes button events without coordinates — we inject the last
            // known position here so hit-testing in Button/Slider works correctly.
            if let Event::MouseMove { pos } = e {
                self.mouse_pos = *pos;
            }

            // Patch mouse events that arrive with pos=(0,0) — substitute the last
            // tracked position so widget hit-testing is correct.
            // parse_mouse_button and parse_mouse_scroll in input_bridge set pos=(0,0)
            // because the input service encodes those events without coordinates.
            let patched;
            let ev: &Event = match e {
                Event::MousePress { pos, button } if pos.x == 0.0 && pos.y == 0.0 => {
                    patched = Event::MousePress { pos: self.mouse_pos, button: *button };
                    &patched
                }
                Event::MouseRelease { pos, button } if pos.x == 0.0 && pos.y == 0.0 => {
                    patched = Event::MouseRelease { pos: self.mouse_pos, button: *button };
                    &patched
                }
                Event::Scroll { pos, delta_y } if pos.x == 0.0 && pos.y == 0.0 => {
                    patched = Event::Scroll { pos: self.mouse_pos, delta_y: *delta_y };
                    &patched
                }
                other => other,
            };

            // Track held key for software repeat injection (see KeyRepeatState).
            match ev {
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

            // Route event: topmost overlay wins, then falls through if non-blocking.
            let consumed = if let Some(top) = self.overlays.last_mut() {
                let top_blocking = top.blocking;
                let overlay_consumed = top.widget.event(ev);
                if top_blocking {
                    // Blocking overlay: never dispatch to root.
                    overlay_consumed
                } else if !overlay_consumed {
                    // Non-blocking overlay did not consume — dispatch to root.
                    self.root.event(ev)
                } else {
                    true
                }
            } else {
                self.root.event(ev)
            };

            // Check dismiss_outside: if a non-blocking overlay's popup should dismiss
            // when clicked outside its bounds, the popup's own event() handles Pop.
            // No additional check needed here.

            if consumed { self.layout_dirty = true; }
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
                    let consumed = if let Some(top) = self.overlays.last_mut() {
                        let top_blocking = top.blocking;
                        let overlay_consumed = top.widget.event(&repeat_event);
                        if top_blocking { overlay_consumed }
                        else if !overlay_consumed { self.root.event(&repeat_event) }
                        else { true }
                    } else {
                        self.root.event(&repeat_event)
                    };
                    if consumed { self.layout_dirty = true; }
                }
            }
        }

        // ── Drain action queue (widgets may have queued actions this frame) ────
        self.drain_action_queue();

        // ── Advance toast timers ──────────────────────────────────────────────
        if dt_ms > 0 && !self.toast_entries.is_empty() {
            let prev_len = self.toast_entries.len();
            for entry in &mut self.toast_entries {
                entry.elapsed_ms = entry.elapsed_ms.saturating_add(dt_ms);
            }
            self.toast_entries.retain(|entry| {
                entry.config.duration_ms == 0
                    || entry.elapsed_ms < entry.config.duration_ms
            });
            if self.toast_entries.len() != prev_len {
                self.layout_dirty = true;
            }
        }

        // ── Path A: structural change → full layout + re-subscribe ────────────
        if self.layout_dirty {
            self.layout_dirty = false;
            let (w, h) = self.renderer.size();
            let screen = Constraints::root(Size::new(w as f32, h as f32));

            // Layout root widget.
            self.root.layout(screen);

            // Layout all overlay widgets (they need fresh screen constraints).
            for entry in &mut self.overlays {
                entry.widget.layout(screen);
            }

            // Layout toast widgets.
            for entry in &mut self.toast_entries {
                entry.widget.layout(screen);
            }

            // Rebuild focus list.  When a blocking overlay is active, focus is
            // confined to that overlay's focusable bounds.
            if self.overlays.last().map(|e| e.blocking).unwrap_or(false) {
                self.focus.list = self.overlays.last_mut()
                    .map(|e| e.widget.collect_focusable_bounds())
                    .unwrap_or_default();
            } else {
                self.focus.list = self.root.collect_focusable_bounds();
            }
            // Clamp idx if the list shrank (e.g. widget removed).
            if let Some(i) = self.focus.idx {
                if i >= self.focus.list.len() { self.focus.idx = None; }
            }

            // Rebuild signal subscriptions for root + overlays.
            self.dirty_handles = self.root.collect_dirty_handles(Rc::clone(&self.dirty_region));
            for entry in &mut self.overlays {
                let handles = entry.widget.collect_dirty_handles(Rc::clone(&self.dirty_region));
                self.dirty_handles.extend(handles);
            }

            self.dirty_region.borrow_mut().mark_all(w as f32, h as f32);
        }

        // ── Path B: render accumulated dirty region ───────────────────────────
        let damage = self.dirty_region.borrow_mut().take();
        if damage.is_none() { return false; }

        // Snapshot focus bounds before the borrow split — Option<Rect> is Copy.
        let focus_bounds = self.focus.current_bounds();

        // Split borrows: renderer, font_ctx, theme, root, overlays, and
        // toast_entries are all disjoint struct fields.
        let renderer      = &mut self.renderer;
        let font_ctx      = &mut self.font_ctx;
        let theme         = &*self.theme;
        let root          = &self.root;
        let overlays      = &self.overlays;
        let toast_entries = &self.toast_entries;

        renderer.render(damage, &mut |canvas| {
            let mut cx = RenderCtx { canvas, font: font_ctx, theme };
            root.paint(&mut cx);

            // Paint overlays bottom-to-top.
            for entry in overlays.iter() {
                if entry.blocking {
                    // Dim the background with a semi-transparent scrim.
                    let (sw, sh) = (cx.canvas.width() as f32, cx.canvas.height() as f32);
                    cx.canvas.fill_rect(
                        crate::layout::Rect::new(0.0, 0.0, sw, sh),
                        crate::canvas::Color::bgra(0, 0, 0, 160),
                    );
                }
                entry.widget.paint(&mut cx);
            }

            // Paint toasts above all overlays.
            for entry in toast_entries.iter() {
                entry.widget.paint(&mut cx);
            }

            // Post-paint focus ring — drawn on top of all widget content.
            if let Some(b) = focus_bounds {
                cx.canvas.draw_rect_border(b, cx.theme.accent(), 2.0);
            }
        });
        true
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    /// Drain the action queue and apply each `OverlayAction` to `self`.
    ///
    /// Called twice per frame: once before event processing (to apply actions
    /// from the previous frame) and once after (to apply actions queued this
    /// frame). The double drain ensures single-frame round-trip latency.
    fn drain_action_queue(&mut self) {
        // Collect into a local vec to release the borrow on action_queue before
        // mutating `self.overlays` / `self.toast_entries`.
        let actions: Vec<OverlayAction> = self.action_queue.borrow_mut().drain(..).collect();
        for action in actions {
            match action {
                OverlayAction::Push(entry) => {
                    self.overlays.push(entry);
                    self.layout_dirty = true;
                }
                OverlayAction::Pop => {
                    self.overlays.pop();
                    self.layout_dirty = true;
                }
                OverlayAction::PopAll => {
                    self.overlays.clear();
                    self.layout_dirty = true;
                }
                OverlayAction::ShowToast(config) => {
                    let widget = crate::node_widgets::toast::Toast::new(&config);
                    self.toast_entries.push_back(crate::node_widgets::toast::ToastEntry {
                        config,
                        elapsed_ms: 0,
                        widget,
                    });
                    self.layout_dirty = true;
                }
            }
        }
    }

    /// Force a full repaint on the next `tick()` call.
    pub fn mark_dirty(&mut self) { self.layout_dirty = true; }
}
