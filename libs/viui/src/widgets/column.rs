//! Column widget — vertically stacks children with configurable spacing.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::event::{Event, EventCx, EventStatus};
use crate::layout::{Constraints, LayoutNode, Padding, Point, Rect, Size};
use crate::widget::{PaintCx, ViWidget};

pub struct Column {
    children: Vec<Box<dyn ViWidget>>,
    spacing:  f32,
    padding:  Padding,
}

impl Column {
    pub fn new(children: Vec<Box<dyn ViWidget>>) -> Self {
        Self { children, spacing: 4.0, padding: Padding::ZERO }
    }
    pub fn spacing(mut self, px: f32) -> Self { self.spacing = px; self }
    pub fn padding(mut self, p: Padding) -> Self { self.padding = p; self }
}

impl ViWidget for Column {
    fn layout(&self, constraints: Constraints) -> LayoutNode {
        let ox = constraints.origin.x + self.padding.left;
        let mut oy = constraints.origin.y + self.padding.top;
        let inner_w = (constraints.max.w - self.padding.h_total()).max(0.0);
        let inner_h = (constraints.max.h - self.padding.v_total()).max(0.0);
        let mut child_nodes = Vec::new();

        for child in &self.children {
            let remaining = (constraints.origin.y + inner_h - oy).max(0.0);
            let child_c = Constraints::new(Point::new(ox, oy), Size { w: inner_w, h: remaining });
            let node = child.layout(child_c);
            let h = node.bounds.h;
            child_nodes.push(node);
            oy += h + self.spacing;
        }

        let total_h = (oy - constraints.origin.y - self.spacing).max(0.0) + self.padding.v_total();
        let bounds = Rect {
            x: constraints.origin.x,
            y: constraints.origin.y,
            w: constraints.max.w,
            h: total_h.min(constraints.max.h),
        };
        LayoutNode::with_children(bounds, child_nodes)
    }

    fn paint(&self, cx: &mut PaintCx) {
        // Each child's origin is already baked into layout bounds (absolute).
        // PaintCx origin = parent origin; child paints at its own absolute origin.
        for child in &self.children { child.paint(cx); }
    }

    fn event(&mut self, cx: &mut EventCx, e: &Event) -> EventStatus {
        for (i, child) in self.children.iter_mut().enumerate() {
            let child_layout = match cx.layout.child(i) { Some(l) => l, None => break };
            let mut child_cx = EventCx {
                state: cx.state,
                focus: cx.focus,
                widget_id: cx.widget_id.child(i),
                layout: child_layout,
                needs_repaint: false,
            };
            let status = child.event(&mut child_cx, e);
            if child_cx.needs_repaint { cx.mark_dirty(); }
            if status == EventStatus::Consumed { return EventStatus::Consumed; }
        }
        EventStatus::Ignored
    }
}
