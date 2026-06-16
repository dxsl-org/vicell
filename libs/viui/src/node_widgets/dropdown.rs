// SPDX-License-Identifier: MIT
//! Drop-down selector widget.
//!
//! Renders a labelled trigger button showing the selected value. When clicked,
//! a `DropDownPopup` is pushed as a non-blocking overlay via the action queue.
//! The popup lays out below (or above, if there is no room) the trigger button
//! and dismisses itself on item selection or outside click.
//!
//! # Usage
//!
//! ```rust,ignore
//! let selected = Signal::new("Option A".into());
//! let queue    = app.action_queue();
//! let dd = DropDown::new(
//!     selected.clone(),
//!     alloc::vec!["Option A".into(), "Option B".into(), "Option C".into()],
//!     queue,
//! );
//! ```

extern crate alloc;
use alloc::{boxed::Box, string::String, vec::Vec};
use core::cell::Cell;

use crate::canvas::Color;
use crate::dirty::DirtyRegion;
use crate::event::{Event, MouseButton};
use crate::layout::{Constraints, Point, Rect, Size};
use crate::node::ViNode;
use crate::overlay::{OverlayAction, OverlayActionQueue, OverlayEntry};
use crate::render_ctx::RenderCtx;
use crate::signal::{Signal, SubscriptionHandle};

// ─── DropDown ────────────────────────────────────────────────────────────────

/// Trigger widget for a drop-down selector.
///
/// Lays out as a fixed-height bar that shows the selected value and a
/// down-arrow indicator. A click pushes `DropDownPopup` as an overlay.
pub struct DropDown {
    selected:  Signal<String>,
    items:     Vec<String>,
    queue:     OverlayActionQueue,
    bounds:    Cell<Rect>,
    hovered:   bool,
}

impl DropDown {
    /// Create a drop-down with the given reactive `selected` value and item list.
    ///
    /// `queue` must be the clone obtained from `ViApp::action_queue()`.
    pub fn new(
        selected: Signal<String>,
        items:    Vec<String>,
        queue:    OverlayActionQueue,
    ) -> Self {
        Self {
            selected,
            items,
            queue,
            bounds:  Cell::new(Rect::ZERO),
            hovered: false,
        }
    }

    /// Open the popup by pushing a `DropDownPopup` overlay.
    ///
    /// Uses the current `bounds` as the anchor so the popup can position itself
    /// relative to the trigger. Called inside `event()` — `bounds` is already
    /// valid from the most recent layout pass.
    fn open_popup(&self) {
        let popup = DropDownPopup::new(
            self.items.clone(),
            self.selected.clone(),
            self.bounds.get(),
            self.queue.clone(),
        );
        self.queue.borrow_mut().push(OverlayAction::Push(OverlayEntry {
            widget:          Box::new(popup),
            blocking:        false,
            dismiss_outside: true,
            anchor_bounds:   Some(self.bounds.get()),
        }));
    }
}

impl ViNode for DropDown {
    fn layout(&mut self, constraints: Constraints) -> Size {
        let w = constraints.max.w.min(200.0);
        let h = 36.0f32;
        self.bounds.set(Rect {
            x: constraints.origin.x,
            y: constraints.origin.y,
            w,
            h,
        });
        Size::new(w, h)
    }

    fn bounds(&self) -> Rect { self.bounds.get() }

    fn paint(&self, cx: &mut RenderCtx<'_>) {
        let b  = self.bounds.get();
        let bg = if self.hovered { Color::rgb(55, 60, 80) } else { Color::rgb(40, 44, 60) };
        cx.canvas.fill_rect(b, bg);
        cx.canvas.draw_rect_border(b, Color::rgb(80, 85, 110), 1.0);

        let text = self.selected.get();
        cx.draw_text(Point::new(b.x + 8.0, b.y + 10.0), &text, Color::WHITE);

        // Down-arrow indicator (right-aligned).
        cx.draw_text(Point::new(b.x + b.w - 18.0, b.y + 10.0), "v", Color::rgb(150, 150, 180));
    }

    fn event(&mut self, event: &Event) -> bool {
        match event {
            Event::MouseMove { pos } => {
                let was = self.hovered;
                self.hovered = self.bounds.get().contains(*pos);
                was != self.hovered // signal dirty only when state changes
            }
            Event::MousePress { pos, button: MouseButton::Left } => {
                if self.bounds.get().contains(*pos) {
                    self.open_popup();
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn collect_dirty_handles(&mut self, region: DirtyRegion) -> Vec<SubscriptionHandle> {
        let b = self.bounds.get();
        alloc::vec![self.selected.subscribe(move |_| {
            region.borrow_mut().mark(b);
        })]
    }
}

// ─── DropDownPopup ───────────────────────────────────────────────────────────

/// Popup list rendered as a non-blocking overlay below (or above) the trigger.
///
/// Selecting an item updates the `selected` signal and pops itself via the queue.
/// Clicking outside the popup also pops it (`dismiss_outside = true` in the entry).
struct DropDownPopup {
    items:    Vec<String>,
    selected: Signal<String>,
    anchor:   Rect,
    queue:    OverlayActionQueue,
    bounds:   Cell<Rect>,
    hovered:  Option<usize>,
}

impl DropDownPopup {
    fn new(
        items:    Vec<String>,
        selected: Signal<String>,
        anchor:   Rect,
        queue:    OverlayActionQueue,
    ) -> Self {
        Self {
            items,
            selected,
            anchor,
            queue,
            bounds:  Cell::new(Rect::ZERO),
            hovered: None,
        }
    }

    /// Item row height in pixels.
    const ITEM_H: f32 = 32.0;
    /// Maximum popup height before clipping.
    const MAX_H:  f32 = 200.0;
}

impl ViNode for DropDownPopup {
    fn layout(&mut self, constraints: Constraints) -> Size {
        let item_h = Self::ITEM_H;
        let w = self.anchor.w;
        let h = (self.items.len() as f32 * item_h).min(Self::MAX_H);
        let x = self.anchor.x;
        // Prefer below; fall back to above when not enough room.
        let y = if self.anchor.y + self.anchor.h + h <= constraints.max.h {
            self.anchor.y + self.anchor.h
        } else {
            (self.anchor.y - h).max(0.0)
        };
        self.bounds.set(Rect { x, y, w, h });
        Size::new(w, h)
    }

    fn bounds(&self) -> Rect { self.bounds.get() }

    fn paint(&self, cx: &mut RenderCtx<'_>) {
        let b        = self.bounds.get();
        let item_h   = Self::ITEM_H;
        let current  = self.selected.get();

        cx.canvas.fill_rect(b, Color::rgb(35, 38, 52));
        cx.canvas.draw_rect_border(b, Color::rgb(70, 72, 100), 1.0);

        for (i, item) in self.items.iter().enumerate() {
            let iy = b.y + i as f32 * item_h;
            let is_selected = item.as_str() == current.as_str();
            let is_hovered  = self.hovered == Some(i);
            let bg = if is_hovered {
                Color::rgb(80, 120, 220)
            } else if is_selected {
                Color::rgb(60, 100, 180)
            } else {
                Color::rgb(35, 38, 52)
            };
            cx.canvas.fill_rect(Rect { x: b.x, y: iy, w: b.w, h: item_h }, bg);
            cx.draw_text(Point::new(b.x + 8.0, iy + 8.0), item, Color::WHITE);
        }
    }

    fn event(&mut self, event: &Event) -> bool {
        let item_h = Self::ITEM_H;
        let b      = self.bounds.get();

        match event {
            Event::MouseMove { pos } => {
                if b.contains(*pos) {
                    let idx = ((pos.y - b.y) / item_h) as usize;
                    self.hovered = if idx < self.items.len() { Some(idx) } else { None };
                } else {
                    self.hovered = None;
                }
                // Don't consume: non-blocking so root still sees moves.
                false
            }
            Event::MousePress { pos, button: MouseButton::Left } => {
                if b.contains(*pos) {
                    true // inside popup — consume to prevent root from acting
                } else {
                    // Outside click — dismiss_outside in OverlayEntry handles pop.
                    false
                }
            }
            Event::MouseRelease { pos, button: MouseButton::Left } => {
                if b.contains(*pos) {
                    let idx = ((pos.y - b.y) / item_h) as usize;
                    if idx < self.items.len() {
                        self.selected.set(self.items[idx].clone());
                    }
                    self.queue.borrow_mut().push(OverlayAction::Pop);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn collect_dirty_handles(&mut self, _region: DirtyRegion) -> Vec<SubscriptionHandle> {
        Vec::new()
    }
}
