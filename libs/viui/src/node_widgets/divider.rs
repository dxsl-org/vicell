// SPDX-License-Identifier: MIT
//! Divider — horizontal or vertical separator line.

extern crate alloc;
use alloc::vec::Vec;
use core::cell::Cell;

use crate::canvas::Color;
use crate::dirty::DirtyRegion;
use crate::event::Event;
use crate::layout::{Axis, Constraints, Rect, Size};
use crate::node::ViNode;
use crate::render_ctx::RenderCtx;
use crate::signal::SubscriptionHandle;

/// Single-pixel separator line.
///
/// Horizontal variant expands to full container width, 1 px tall.
/// Vertical variant expands to full container height, 1 px wide.
pub struct Divider {
    axis:         Axis,
    color:        Color,
    bounds_cache: Cell<Rect>,
}

impl Divider {
    /// 1 px horizontal rule spanning full width.
    pub fn horizontal() -> Self {
        Self { axis: Axis::Horizontal, color: Color::rgb(60, 60, 80), bounds_cache: Cell::new(Rect::ZERO) }
    }

    /// 1 px vertical rule spanning full height.
    pub fn vertical() -> Self {
        Self { axis: Axis::Vertical, color: Color::rgb(60, 60, 80), bounds_cache: Cell::new(Rect::ZERO) }
    }

    /// Override the default separator color.
    pub fn color(mut self, c: Color) -> Self { self.color = c; self }
}

impl ViNode for Divider {
    fn layout(&mut self, constraints: Constraints) -> Size {
        let size = match self.axis {
            Axis::Horizontal => constraints.constrain(Size { w: constraints.max.w, h: 1.0 }),
            Axis::Vertical   => constraints.constrain(Size { w: 1.0, h: constraints.max.h }),
        };
        self.bounds_cache.set(Rect::from_origin_size(constraints.origin, size));
        size
    }

    fn bounds(&self) -> Rect { self.bounds_cache.get() }

    fn paint(&self, cx: &mut RenderCtx<'_>) {
        cx.canvas.fill_rect(self.bounds_cache.get(), self.color);
    }

    // Dividers are non-interactive.
    fn event(&mut self, _event: &Event) -> bool { false }

    fn collect_dirty_handles(&mut self, _region: DirtyRegion) -> Vec<SubscriptionHandle> {
        alloc::vec![]
    }
}
