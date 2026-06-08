// SPDX-License-Identifier: MIT
//! TouchArea — transparent hit-zone for raw gesture capture.
//!
//! Wraps a child widget and intercepts pointer/touch events before they
//! reach the child. Fires `on_tap` on short press-release, `on_drag` on moves.

extern crate alloc;
use alloc::{boxed::Box, vec::Vec};
use core::cell::Cell;

use crate::dirty::DirtyRegion;
use crate::event::{Event, MouseButton};
use crate::layout::{Constraints, Point, Rect, Size};
use crate::node::ViNode;
use crate::render_ctx::RenderCtx;
use crate::signal::SubscriptionHandle;

/// Pixel distance below which a press-release sequence counts as a tap.
const TAP_THRESHOLD: f32 = 8.0;

/// Transparent gesture layer wrapping a child widget.
///
/// All pointer and touch events within bounds are captured; the child only
/// receives events that TouchArea does not consume.
pub struct TouchArea {
    child:     Box<dyn ViNode>,
    on_tap:    Option<Box<dyn Fn()>>,
    on_drag:   Option<Box<dyn Fn(Point)>>,    // delta from press point
    press_pos: Cell<Option<Point>>,
    last_pos:  Cell<Point>,
}

impl TouchArea {
    pub fn new(child: Box<dyn ViNode>) -> Self {
        Self {
            child,
            on_tap:    None,
            on_drag:   None,
            press_pos: Cell::new(None),
            last_pos:  Cell::new(Point::new(0.0, 0.0)),
        }
    }

    /// Fire `f` when a tap is recognized (short press-release, no drag).
    pub fn on_tap(mut self, f: impl Fn() + 'static) -> Self {
        self.on_tap = Some(Box::new(f));
        self
    }

    /// Fire `f(delta)` continuously while dragging, where `delta` is movement
    /// from the initial press position.
    pub fn on_drag(mut self, f: impl Fn(Point) + 'static) -> Self {
        self.on_drag = Some(Box::new(f));
        self
    }
}

impl ViNode for TouchArea {
    fn layout(&mut self, constraints: Constraints) -> Size {
        self.child.layout(constraints)
    }

    fn bounds(&self) -> Rect { self.child.bounds() }

    fn paint(&self, cx: &mut RenderCtx<'_>) {
        self.child.paint(cx);
    }

    fn event(&mut self, event: &Event) -> bool {
        let bounds = self.child.bounds();

        let handle_begin = |pos: Point| -> bool {
            if bounds.contains(pos) {
                self.press_pos.set(Some(pos));
                self.last_pos.set(pos);
                true
            } else { false }
        };

        let handle_move = |pos: Point| -> bool {
            if self.press_pos.get().is_some() {
                self.last_pos.set(pos);
                if let Some(start) = self.press_pos.get() {
                    let delta = Point::new(pos.x - start.x, pos.y - start.y);
                    if let Some(cb) = &self.on_drag { cb(delta); }
                }
                true
            } else { false }
        };

        let handle_end = |pos: Point| -> bool {
            if let Some(start) = self.press_pos.get().take() {
                self.press_pos.set(None);
                let dx = pos.x - start.x;
                let dy = pos.y - start.y;
                // Squared distance — avoids f32::sqrt() unavailable in no_std
                if dx * dx + dy * dy < TAP_THRESHOLD * TAP_THRESHOLD {
                    if let Some(cb) = &self.on_tap { cb(); }
                }
                true
            } else { false }
        };

        match event {
            Event::MousePress { pos, button: MouseButton::Left } => handle_begin(*pos),
            Event::MouseMove  { pos }                            => handle_move(*pos),
            Event::MouseRelease { pos, button: MouseButton::Left } => handle_end(*pos),
            Event::TouchBegin { pos, finger_id: 0 }             => handle_begin(*pos),
            Event::TouchMove  { pos, finger_id: 0 }             => handle_move(*pos),
            Event::TouchEnd   { pos, finger_id: 0 }             => handle_end(*pos),
            // Pass remaining events to child
            other => self.child.event(other),
        }
    }

    fn collect_dirty_handles(&mut self, region: DirtyRegion) -> Vec<SubscriptionHandle> {
        self.child.collect_dirty_handles(region)
    }
}
