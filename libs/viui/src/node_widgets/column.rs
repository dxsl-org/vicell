// SPDX-License-Identifier: MIT
//! Column widget — stacks children vertically (vstack).

extern crate alloc;
use alloc::{boxed::Box, rc::Rc, vec::Vec};

use crate::dirty::DirtyRegion;
use crate::event::Event;
use crate::layout::{Constraints, Point, Rect, Size};
use crate::node::ViNode;
use crate::render_ctx::RenderCtx;
use crate::signal::SubscriptionHandle;

/// Vertical layout container.
///
/// Children are laid out top-to-bottom with `spacing` between them and
/// `padding` on all sides.
pub struct Column {
    pub children: Vec<Box<dyn ViNode>>,
    pub spacing:  f32,
    pub padding:  f32,
    bounds:       Rect,
}

impl Column {
    pub fn new(children: Vec<Box<dyn ViNode>>) -> Self {
        Self { children, spacing: 4.0, padding: 0.0, bounds: Rect::ZERO }
    }

    pub fn with_spacing(mut self, s: f32) -> Self { self.spacing = s; self }
    pub fn with_padding(mut self, p: f32) -> Self { self.padding = p; self }
}

impl ViNode for Column {
    fn layout(&mut self, constraints: Constraints) -> Size {
        let pad = self.padding;
        let sp  = self.spacing;
        let mut y    = constraints.origin.y + pad;
        let x        = constraints.origin.x + pad;
        let inner_w  = (constraints.max.w - 2.0 * pad).max(0.0);
        let mut used_h = pad;

        for child in &mut self.children {
            let avail_h = (constraints.max.h - used_h - pad).max(0.0);
            let child_size = child.layout(
                Constraints::new(Point::new(x, y), Size { w: inner_w, h: avail_h })
            );
            y      += child_size.h + sp;
            used_h += child_size.h + sp;
        }

        if !self.children.is_empty() { used_h -= sp; }
        used_h += pad;

        let size = constraints.constrain(Size {
            w: constraints.max.w,
            h: used_h,
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
