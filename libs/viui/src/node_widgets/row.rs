// SPDX-License-Identifier: MIT
//! Row widget — stacks children horizontally (hstack).

extern crate alloc;
use alloc::{boxed::Box, rc::Rc, vec::Vec};

use crate::dirty::DirtyRegion;
use crate::event::Event;
use crate::layout::{Constraints, Point, Rect, Size};
use crate::node::ViNode;
use crate::render_ctx::RenderCtx;
use crate::signal::SubscriptionHandle;

/// Horizontal layout container.
///
/// Children are laid out left-to-right with `spacing` between them and
/// `padding` on all sides.
pub struct Row {
    pub children: Vec<Box<dyn ViNode>>,
    pub spacing:  f32,
    pub padding:  f32,
    bounds:       Rect,
}

impl Row {
    pub fn new(children: Vec<Box<dyn ViNode>>) -> Self {
        Self { children, spacing: 4.0, padding: 0.0, bounds: Rect::ZERO }
    }

    pub fn with_spacing(mut self, s: f32) -> Self { self.spacing = s; self }
    pub fn with_padding(mut self, p: f32) -> Self { self.padding = p; self }
}

impl ViNode for Row {
    fn layout(&mut self, constraints: Constraints) -> Size {
        let pad = self.padding;
        let sp  = self.spacing;
        let mut x    = constraints.origin.x + pad;
        let y        = constraints.origin.y + pad;
        let inner_h  = (constraints.max.h - 2.0 * pad).max(0.0);
        let mut used_w = pad;

        for child in &mut self.children {
            let avail_w = (constraints.max.w - used_w - pad).max(0.0);
            let child_size = child.layout(
                Constraints::new(Point::new(x, y), Size { w: avail_w, h: inner_h })
            );
            x      += child_size.w + sp;
            used_w += child_size.w + sp;
        }

        if !self.children.is_empty() { used_w -= sp; }
        used_w += pad;

        let size = constraints.constrain(Size {
            w: used_w,
            h: constraints.max.h,
        });
        self.bounds = Rect::from_origin_size(constraints.origin, size);
        size
    }

    fn bounds(&self) -> Rect { self.bounds }

    fn paint(&self, cx: &mut RenderCtx<'_>) {
        for child in &self.children {
            child.paint(cx);
        }
    }

    fn event(&mut self, event: &Event) -> bool {
        for child in self.children.iter_mut().rev() {
            if child.event(event) { return true; }
        }
        false
    }

    fn collect_dirty_handles(&mut self, region: DirtyRegion) -> Vec<SubscriptionHandle> {
        let mut handles = Vec::new();
        for child in &mut self.children {
            handles.extend(child.collect_dirty_handles(Rc::clone(&region)));
        }
        handles
    }
}
