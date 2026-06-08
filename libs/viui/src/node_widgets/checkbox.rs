// SPDX-License-Identifier: MIT
//! CheckBox — toggleable boolean widget with optional text label.

extern crate alloc;
use alloc::{boxed::Box, string::String, vec::Vec};
use core::cell::Cell;

use crate::canvas::Color;
use crate::dirty::DirtyRegion;
use crate::event::{Event, MouseButton};
use crate::layout::{Constraints, Point, Rect, Size};
use crate::node::ViNode;
use crate::render_ctx::RenderCtx;
use crate::signal::{Signal, SubscriptionHandle};

/// Total widget height in pixels.
const HEIGHT:   f32 = 24.0;
/// Side length of the toggle box.
const BOX_SIZE: f32 = 16.0;
/// Horizontal gap between box right edge and label text.
const LABEL_GAP: f32 = 6.0;

/// Boolean toggle with a checkmark glyph and an optional text label.
///
/// Clicking anywhere in the widget bounds inverts `checked`.
/// `on_toggle` fires after the state change with the new value.
pub struct CheckBox {
    /// Current checked state.
    pub checked:  Signal<bool>,
    /// Label shown to the right of the box. Empty string hides the label.
    pub label:    Signal<String>,
    on_toggle:    Option<Box<dyn Fn(bool)>>,
    bounds_cache: Cell<Rect>,
}

impl CheckBox {
    pub fn new(checked: Signal<bool>) -> Self {
        Self {
            checked,
            label:        Signal::new(String::new()),
            on_toggle:    None,
            bounds_cache: Cell::new(Rect::ZERO),
        }
    }

    /// Attach a reactive label signal.
    pub fn label(mut self, s: Signal<String>) -> Self { self.label = s; self }

    /// Attach a static label string.
    pub fn label_str(self, s: impl Into<String>) -> Self {
        self.label(Signal::new(s.into()))
    }

    /// Callback fired with the new `bool` value after every toggle.
    pub fn on_toggle(mut self, f: impl Fn(bool) + 'static) -> Self {
        self.on_toggle = Some(Box::new(f));
        self
    }
}

impl ViNode for CheckBox {
    fn layout(&mut self, constraints: Constraints) -> Size {
        let size = constraints.constrain(Size { w: constraints.max.w, h: HEIGHT });
        self.bounds_cache.set(Rect::from_origin_size(constraints.origin, size));
        size
    }

    fn bounds(&self) -> Rect { self.bounds_cache.get() }

    fn paint(&self, cx: &mut RenderCtx<'_>) {
        let b = self.bounds_cache.get();

        // Vertically center the box within the widget height.
        let box_y    = b.y + (b.h - BOX_SIZE) * 0.5;
        let box_rect = Rect { x: b.x + 2.0, y: box_y, w: BOX_SIZE, h: BOX_SIZE };

        // Box fill
        cx.canvas.fill_rect(box_rect, Color::rgb(30, 30, 45));

        // Box border — 4 edge lines
        let bc = Color::rgb(100, 100, 130);
        let x0 = box_rect.x;
        let y0 = box_rect.y;
        let x1 = x0 + box_rect.w;
        let y1 = y0 + box_rect.h;
        cx.canvas.draw_line(Point::new(x0, y0), Point::new(x1, y0), bc); // top
        cx.canvas.draw_line(Point::new(x1, y0), Point::new(x1, y1), bc); // right
        cx.canvas.draw_line(Point::new(x1, y1), Point::new(x0, y1), bc); // bottom
        cx.canvas.draw_line(Point::new(x0, y1), Point::new(x0, y0), bc); // left

        // Checkmark: two-segment ✓ path when checked
        if *self.checked.get() {
            let accent = Color::rgb(80, 140, 220);
            // Left segment: bottom-left corner towards center-bottom
            let start   = Point::new(x0 + 3.0,            y0 + BOX_SIZE * 0.55);
            let mid     = Point::new(x0 + BOX_SIZE * 0.35, y1 - 3.0);
            // Right segment: center-bottom up to top-right
            let end     = Point::new(x1 - 3.0,             y0 + 4.0);
            cx.canvas.draw_line(start, mid, accent);
            cx.canvas.draw_line(mid,   end, accent);
        }

        // Label text (if non-empty)
        let label = self.label.get();
        if !label.is_empty() {
            let lx = b.x + 2.0 + BOX_SIZE + LABEL_GAP;
            let ly = b.y + (b.h - cx.line_height()) * 0.5;
            cx.draw_text(Point::new(lx, ly.max(b.y)), &label, Color::rgb(220, 220, 230));
        }
    }

    fn event(&mut self, event: &Event) -> bool {
        let b = self.bounds_cache.get();

        let toggled = match event {
            Event::MousePress  { pos, button: MouseButton::Left } if b.contains(*pos) => true,
            Event::TouchBegin  { pos, .. }                        if b.contains(*pos) => true,
            _ => false,
        };

        if toggled {
            let new_val = !*self.checked.get();
            self.checked.set(new_val);
            if let Some(cb) = &self.on_toggle { cb(new_val); }
        }

        toggled
    }

    fn collect_dirty_handles(&mut self, region: DirtyRegion) -> Vec<SubscriptionHandle> {
        let bounds   = self.bounds_cache.get();
        let region2  = region.clone();
        let h1 = self.checked.subscribe(move || { region.borrow_mut().mark(bounds); });
        let h2 = self.label.subscribe(move || { region2.borrow_mut().mark(bounds); });
        alloc::vec![h1, h2]
    }
}
