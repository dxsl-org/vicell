// SPDX-License-Identifier: MIT
//! Card — panel container with a background fill, border, and inner padding.

extern crate alloc;
use alloc::{boxed::Box, vec::Vec};
use core::cell::Cell;

use crate::canvas::Color;
use crate::dirty::DirtyRegion;
use crate::event::Event;
use crate::layout::{Constraints, Point, Rect, Size};
use crate::node::ViNode;
use crate::render_ctx::RenderCtx;
use crate::signal::SubscriptionHandle;

/// Container that draws a tinted panel behind its child widget.
///
/// `padding` shrinks the child's allocated area on all sides.
/// The child's bounds are set relative to the card's padded interior.
pub struct Card {
    child:        Box<dyn ViNode>,
    padding:      f32,
    bg_color:     Color,
    border_color: Color,
    bounds_cache: Cell<Rect>,
}

impl Card {
    /// Wrap `child` with default padding (8 px), dark background, and subtle border.
    pub fn new(child: Box<dyn ViNode>) -> Self {
        Self {
            child,
            padding:      8.0,
            bg_color:     Color::rgb(30, 30, 45),
            border_color: Color::rgb(60, 60, 80),
            bounds_cache: Cell::new(Rect::ZERO),
        }
    }

    /// Uniform padding on all sides.
    pub fn padding(mut self, p: f32) -> Self { self.padding = p; self }

    /// Background fill color.
    pub fn bg(mut self, c: Color) -> Self { self.bg_color = c; self }

    /// Border color.
    pub fn border(mut self, c: Color) -> Self { self.border_color = c; self }
}

impl ViNode for Card {
    fn layout(&mut self, constraints: Constraints) -> Size {
        let pad = self.padding;

        // Give child the interior area — origin shifted in by padding.
        let inner = Constraints {
            origin: Point::new(constraints.origin.x + pad, constraints.origin.y + pad),
            min:    Size { w: 0.0, h: 0.0 },
            max:    Size {
                w: (constraints.max.w - pad * 2.0).max(0.0),
                h: (constraints.max.h - pad * 2.0).max(0.0),
            },
        };
        let inner_size = self.child.layout(inner);

        let size = constraints.constrain(Size {
            w: inner_size.w + pad * 2.0,
            h: inner_size.h + pad * 2.0,
        });
        self.bounds_cache.set(Rect::from_origin_size(constraints.origin, size));
        size
    }

    fn bounds(&self) -> Rect { self.bounds_cache.get() }

    fn paint(&self, cx: &mut RenderCtx<'_>) {
        let b = self.bounds_cache.get();

        // Background fill
        cx.canvas.fill_rect(b, self.bg_color);

        // 1 px border — 4 edge lines
        let x0 = b.x;
        let y0 = b.y;
        let x1 = b.x + b.w;
        let y1 = b.y + b.h;
        cx.canvas.draw_line(Point::new(x0, y0), Point::new(x1, y0), self.border_color); // top
        cx.canvas.draw_line(Point::new(x1, y0), Point::new(x1, y1), self.border_color); // right
        cx.canvas.draw_line(Point::new(x1, y1), Point::new(x0, y1), self.border_color); // bottom
        cx.canvas.draw_line(Point::new(x0, y1), Point::new(x0, y0), self.border_color); // left

        // Child paints inside the padded interior
        self.child.paint(cx);
    }

    fn event(&mut self, event: &Event) -> bool {
        self.child.event(event)
    }

    fn collect_dirty_handles(&mut self, region: DirtyRegion) -> Vec<SubscriptionHandle> {
        self.child.collect_dirty_handles(region)
    }
}
