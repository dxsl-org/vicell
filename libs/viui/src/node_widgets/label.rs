// SPDX-License-Identifier: MIT
//! Label v2 — text widget driven by a `Signal<String>`.

use crate::canvas::Color;
use crate::dirty::DirtyRegion;
use crate::event::Event;
use crate::layout::{Constraints, Rect, Size};
use crate::node::ViNode;
use crate::render_ctx::RenderCtx;
use crate::signal::{Signal, SubscriptionHandle};

extern crate alloc;
use alloc::{string::String, vec::Vec};

/// Text display widget.
///
/// `text` is a `Signal<String>` — call `signal.set()` from anywhere to update
/// the displayed text. The app runner will repaint on the next tick.
pub struct Label {
    pub text:  Signal<String>,
    pub color: Color,
    bounds:    Rect,
    cached_byte_len:   usize,
    cached_char_count: usize,
}

impl Label {
    pub fn new(text: Signal<String>) -> Self {
        Self { text, color: Color::WHITE, bounds: Rect::ZERO, cached_byte_len: 0, cached_char_count: 0 }
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }
}

impl ViNode for Label {
    fn layout(&mut self, constraints: Constraints) -> Size {
        let byte_len = self.text.get().len();
        if byte_len != self.cached_byte_len {
            self.cached_char_count = self.text.get().chars().count();
            self.cached_byte_len = byte_len;
        }
        // Use 8.0px char width as conservative fallback; RenderCtx provides
        // better metrics at paint time but layout doesn't have cx access.
        let desired = Size {
            w: self.cached_char_count as f32 * 8.0,
            h: 16.0,   // accommodate scalable font (no longer hard-coded 8px)
        };
        let size = constraints.constrain(desired);
        self.bounds = Rect::from_origin_size(constraints.origin, size);
        size
    }

    fn bounds(&self) -> Rect { self.bounds }

    fn paint(&self, cx: &mut RenderCtx<'_>) {
        let pos = crate::layout::Point::new(self.bounds.x, self.bounds.y);
        cx.draw_text(pos, &self.text.get(), self.color);
    }

    fn event(&mut self, _event: &Event) -> bool { false }

    fn collect_dirty_handles(&mut self, region: DirtyRegion) -> Vec<SubscriptionHandle> {
        let rect = self.bounds;
        let h = self.text.subscribe(move || { region.borrow_mut().mark(rect); });
        alloc::vec![h]
    }
}
