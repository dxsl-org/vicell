// SPDX-License-Identifier: MIT
//! ListView — scrollable list driven by `Signal<Vec<String>>`.
//!
//! Non-virtual render (G1): renders only the visible row range via clip rect.
//! For n > 200 items, upgrade to Phase 08 virtual render.
//!
//! # Scroll behaviour
//! Mouse-wheel and touch-scroll update `scroll_offset`.  A 4px scrollbar thumb
//! is drawn when content exceeds the visible height.
//!
//! # Selection
//! Mouse-click or touch-begin selects the item under the pointer and fires
//! `on_select(idx)`.  `selected` is a readable/writable `Signal<Option<usize>>`
//! so callers can also set the selection programmatically.

extern crate alloc;
use alloc::{boxed::Box, vec::Vec};
use core::cell::Cell;

use crate::dirty::DirtyRegion;
use crate::event::{Event, MouseButton};
use crate::layout::{Constraints, Point, Rect, Size};
use crate::node::ViNode;
use crate::render_ctx::RenderCtx;
use crate::signal::{Signal, SubscriptionHandle};

const DEFAULT_ITEM_H: f32 = 28.0;
const SCROLL_SPEED: f32 = 3.0;

/// Scrollable list of string items driven by a `Signal<Vec<String>>`.
///
/// # Invariants
/// - `scroll_offset` is always clamped to `[0.0, max_scroll()]`.
/// - `bounds_cache` holds the last rect from `layout()`; `Rect::ZERO` before first layout.
pub struct ListView {
    /// The list of items to display.
    pub items: Signal<Vec<alloc::string::String>>,
    /// Currently selected item index, or `None`.
    pub selected: Signal<Option<usize>>,
    on_select: Option<Box<dyn Fn(usize)>>,
    item_height: f32,
    scroll_offset: Cell<f32>,
    bounds_cache: Cell<Rect>,
}

impl ListView {
    /// Create a new `ListView` driven by the given items signal.
    pub fn new(items: Signal<Vec<alloc::string::String>>) -> Self {
        Self {
            items,
            selected: Signal::new(None),
            on_select: None,
            item_height: DEFAULT_ITEM_H,
            scroll_offset: Cell::new(0.0),
            bounds_cache: Cell::new(Rect::ZERO),
        }
    }

    /// Override the per-item height in pixels (default: 28).
    pub fn item_height(mut self, h: f32) -> Self {
        self.item_height = h;
        self
    }

    /// Register a callback fired with the selected item index on each selection change.
    pub fn on_select(mut self, f: impl Fn(usize) + 'static) -> Self {
        self.on_select = Some(Box::new(f));
        self
    }

    /// Replace the selection signal (allows external control of selected item).
    pub fn with_selected(mut self, sel: Signal<Option<usize>>) -> Self {
        self.selected = sel;
        self
    }

    /// Maximum valid scroll offset given current bounds and item count.
    fn max_scroll(&self) -> f32 {
        let b = self.bounds_cache.get();
        let item_count = self.items.get().len();
        let content_h = item_count as f32 * self.item_height;
        (content_h - b.h).max(0.0)
    }

    /// Return the item index at screen position `pos`, or `None` if outside bounds.
    fn item_at(&self, pos: Point) -> Option<usize> {
        let b = self.bounds_cache.get();
        if !b.contains(pos) {
            return None;
        }
        let rel_y = pos.y - b.y + self.scroll_offset.get();
        let idx = (rel_y / self.item_height) as usize;
        let len = self.items.get().len();
        if idx < len { Some(idx) } else { None }
    }
}

impl ViNode for ListView {
    fn layout(&mut self, constraints: Constraints) -> Size {
        // Constrain height to a reasonable maximum; full width.
        let desired_h = constraints.max.h.min(200.0);
        let size = constraints.constrain(Size { w: constraints.max.w, h: desired_h });
        self.bounds_cache.set(Rect::from_origin_size(constraints.origin, size));
        size
    }

    fn bounds(&self) -> Rect {
        self.bounds_cache.get()
    }

    fn paint(&self, cx: &mut RenderCtx<'_>) {
        let b = self.bounds_cache.get();
        let scroll = self.scroll_offset.get();

        // Background
        cx.canvas.fill_rect(b, cx.theme.bg());

        cx.canvas.clip_push(b);

        {
            // Hold a borrow of the items vec for the duration of the visible-range scan.
            let items = self.items.get();
            let sel = *self.selected.get();

            // Only render the visible row range.
            // `as usize` truncates toward zero (floor for positive values).
            // Adding 2 to visible_rows ensures partial items at top/bottom are drawn.
            let first = (scroll / self.item_height) as usize;
            let visible_rows = (b.h / self.item_height) as usize + 2;
            let last = (first + visible_rows).min(items.len());

            for i in first..last {
                let item_y = b.y + i as f32 * self.item_height - scroll;
                let item_rect = Rect { x: b.x, y: item_y, w: b.w, h: self.item_height };

                if sel == Some(i) {
                    cx.canvas.fill_rect(item_rect, cx.theme.list_selected_bg());
                }

                if let Some(text) = items.get(i) {
                    let line_h = cx.line_height();
                    let ty = (item_y + (self.item_height - line_h) * 0.5).max(b.y);
                    let text_color = if sel == Some(i) { cx.theme.list_selected_fg() } else { cx.theme.text_primary() };
                    cx.draw_text(Point::new(b.x + 6.0, ty), text, text_color);
                }
            }
        }

        cx.canvas.clip_pop();

        // Draw scrollbar when content overflows.
        let content_h = self.items.get().len() as f32 * self.item_height;
        if content_h > b.h {
            let bar_w = 4.0_f32;
            let bar_x = b.x + b.w - bar_w;
            let thumb_h = (b.h / content_h * b.h).max(20.0);
            let scroll_range = content_h - b.h;
            let thumb_y = if scroll_range > 0.0 {
                b.y + (scroll / scroll_range) * (b.h - thumb_h)
            } else {
                b.y
            };
            let thumb_y = thumb_y.min(b.y + b.h - thumb_h);

            cx.canvas.fill_rect(
                Rect { x: bar_x, y: b.y, w: bar_w, h: b.h },
                cx.theme.surface(),
            );
            cx.canvas.fill_rect(
                Rect { x: bar_x, y: thumb_y, w: bar_w, h: thumb_h },
                cx.theme.border(),
            );
        }
    }

    fn event(&mut self, event: &Event) -> bool {
        let b = self.bounds_cache.get();
        match event {
            Event::Scroll { pos, delta_y } if b.contains(*pos) => {
                let new_off = (self.scroll_offset.get() - delta_y * SCROLL_SPEED)
                    .clamp(0.0, self.max_scroll());
                self.scroll_offset.set(new_off);
                true
            }
            Event::MousePress { pos, button: MouseButton::Left } => {
                if let Some(idx) = self.item_at(*pos) {
                    self.selected.set(Some(idx));
                    if let Some(cb) = &self.on_select {
                        cb(idx);
                    }
                    true
                } else {
                    false
                }
            }
            Event::TouchBegin { pos, .. } => {
                if let Some(idx) = self.item_at(*pos) {
                    self.selected.set(Some(idx));
                    if let Some(cb) = &self.on_select {
                        cb(idx);
                    }
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Subscribe items + selected signals; mark bounds dirty on any change.
    fn collect_dirty_handles(&mut self, region: DirtyRegion) -> Vec<SubscriptionHandle> {
        let bounds = self.bounds_cache.get();
        let region2 = region.clone();
        let h1 = self.items.subscribe(move || {
            region.borrow_mut().mark(bounds);
        });
        let h2 = self.selected.subscribe(move || {
            region2.borrow_mut().mark(bounds);
        });
        alloc::vec![h1, h2]
    }
}
