// SPDX-License-Identifier: MIT
//! ScrollArea — vertically scrollable container wrapping any `ViNode` child.
//!
//! # Scroll implementation notes
//!
//! `ViCanvas` has no `translate()` method (G1 constraint). Scroll is implemented
//! by shifting the child's layout origin by `-scroll_y` during the layout pass so
//! the child's cached bounds are already in the correct (scrolled) screen position.
//! `clip_push` hides out-of-viewport content.
//!
//! This means `layout()` must be re-run whenever `scroll_y` changes. `ViApp`
//! already re-lays out on every frame when dirty — so a scroll event marks the
//! region dirty and the subsequent layout + paint shows the shifted content.
//!
//! # Scrollbar
//!
//! A 6 px-wide thumb scrollbar is drawn on the right edge when content overflows.

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

const SCROLL_SPEED: f32 = 30.0;
const SCROLLBAR_W:  f32 = 6.0;
const THUMB_MIN_H:  f32 = 24.0;

pub struct ScrollArea {
    child:          Box<dyn ViNode>,
    /// Current vertical scroll offset in pixels (top of visible window).
    scroll_y:       Cell<f32>,
    /// Full unclipped height of the child content.
    content_height: Cell<f32>,
    bounds_cache:   Cell<Rect>,
}

impl ScrollArea {
    pub fn new(child: Box<dyn ViNode>) -> Self {
        Self {
            child,
            scroll_y:       Cell::new(0.0),
            content_height: Cell::new(0.0),
            bounds_cache:   Cell::new(Rect::ZERO),
        }
    }

    /// Maximum scroll offset — zero when content fits in the viewport.
    fn max_scroll(&self) -> f32 {
        (self.content_height.get() - self.bounds_cache.get().h).max(0.0)
    }
}

impl ViNode for ScrollArea {
    fn layout(&mut self, constraints: Constraints) -> Size {
        let size = constraints.constrain(Size {
            w: constraints.max.w,
            h: constraints.max.h,
        });
        self.bounds_cache.set(Rect::from_origin_size(constraints.origin, size));

        let scroll = self.scroll_y.get();

        // Child gets full width minus scrollbar, unbounded height.
        // Origin is shifted up by scroll_y so child bounds are already scrolled.
        let child_constraints = Constraints {
            origin: Point::new(
                constraints.origin.x,
                constraints.origin.y - scroll,
            ),
            min: Size { w: 0.0, h: 0.0 },
            max: Size {
                w: (size.w - SCROLLBAR_W).max(0.0),
                // Unlimited height — measure the full content.
                h: f32::MAX,
            },
        };
        let child_size = self.child.layout(child_constraints);
        self.content_height.set(child_size.h);

        size
    }

    fn bounds(&self) -> Rect { self.bounds_cache.get() }

    fn paint(&self, cx: &mut RenderCtx<'_>) {
        let b      = self.bounds_cache.get();
        let scroll = self.scroll_y.get();

        // Clip to viewport — hides child content that scrolled out of view
        cx.canvas.clip_push(b);

        // Child's bounds are already shifted by -scroll_y (set during layout).
        // Paint proceeds normally; clipping hides anything outside the viewport.
        self.child.paint(cx);

        cx.canvas.clip_pop();

        // Scrollbar — only rendered when content overflows
        let content_h = self.content_height.get();
        if content_h > b.h {
            let track = Rect { x: b.x + b.w - SCROLLBAR_W, y: b.y, w: SCROLLBAR_W, h: b.h };
            cx.canvas.fill_rect(track, Color::rgb(25, 25, 38));

            let max_scroll  = (content_h - b.h).max(1.0);
            let thumb_ratio = b.h / content_h;
            let thumb_h     = (thumb_ratio * b.h).max(THUMB_MIN_H).min(b.h);
            let thumb_top   = b.y + (scroll / max_scroll) * (b.h - thumb_h);
            let thumb = Rect {
                x: track.x + 1.0,
                y: thumb_top.clamp(b.y, b.y + b.h - thumb_h),
                w: SCROLLBAR_W - 2.0,
                h: thumb_h,
            };
            cx.canvas.fill_rect(thumb, Color::rgb(80, 80, 110));
        }
    }

    fn event(&mut self, event: &Event) -> bool {
        let b = self.bounds_cache.get();

        match event {
            Event::Scroll { pos, delta_y } if b.contains(*pos) => {
                // delta_y: negative = scroll down (conventional mouse wheel direction)
                let new_y = (self.scroll_y.get() - delta_y * SCROLL_SPEED)
                    .clamp(0.0, self.max_scroll());
                self.scroll_y.set(new_y);
                true
            }
            _ => self.child.event(event),
        }
    }

    fn collect_dirty_handles(&mut self, region: DirtyRegion) -> Vec<SubscriptionHandle> {
        self.child.collect_dirty_handles(region)
    }
}
