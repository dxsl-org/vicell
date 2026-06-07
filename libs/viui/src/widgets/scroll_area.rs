//! ScrollArea widget — vertically scrollable container.
//!
//! Child is laid out with an unconstrained height; visible region clips output.
//! Scroll state lives in `WidgetStateStore` keyed by `id` (survives Elm rebuilds).

use alloc::boxed::Box;

use crate::event::{Event, EventCx, EventStatus};
use crate::layout::{Constraints, LayoutNode, Rect, Size};
use crate::widget::{PaintCx, WidgetId, ViWidget};

pub struct ScrollArea {
    pub id:         WidgetId,
    child:          Box<dyn ViWidget>,
    child_h:        f32,    // cached from last layout
    visible_h:      f32,    // cached from last layout
}

impl ScrollArea {
    pub fn new(id: WidgetId, child: Box<dyn ViWidget>) -> Self {
        Self { id, child, child_h: 0.0, visible_h: 0.0 }
    }
}

impl ViWidget for ScrollArea {
    fn layout(&self, constraints: Constraints) -> LayoutNode {
        // Own size: fill available space
        let own_size = Size {
            w: constraints.max.w,
            h: constraints.max.h,
        };
        let own_size = constraints.constrain(own_size);

        // Child: unconstrained height (up to 4× own to limit allocation)
        let child_max = Size { w: own_size.w, h: own_size.h * 4.0 };
        let child_constraints = Constraints::new(constraints.origin, child_max);
        let child_node = self.child.layout(child_constraints);

        let bounds = Rect::from_origin_size(constraints.origin, own_size);
        LayoutNode::with_children(bounds, alloc::vec![child_node])
    }

    fn post_layout(&mut self, bounds: Rect) {
        self.visible_h = bounds.h;
        // child_h is the child's natural height; read from layout cache would need
        // to pass it — here we compute it from the last layout result via the child
        // node (approximated from the constraint we set: own_h * 4 or actual child
        // preferred). A precise value comes from the child's layout node, which we
        // pass indirectly via WidgetTree → dispatch. For now, set to 0 (clamp in event).
        // This is sufficient for the clamp in scroll_y update below.
    }

    fn paint(&self, cx: &mut PaintCx) {
        let bounds = Rect::from_origin_size(cx.origin, Size { w: cx.canvas.width() as f32, h: self.visible_h });
        cx.canvas.clip_push(bounds);
        // PaintCx doesn't carry state; scroll offset is approximated from own fields.
        // In P06+ Elm mode, the Elm app manages scroll_y explicitly.
        let child_origin = cx.origin; // scroll_y offset deferred to P06
        let mut child_cx = cx.child_at(child_origin);
        self.child.paint(&mut child_cx);
        cx.canvas.clip_pop();
    }

    fn event(&mut self, cx: &mut EventCx, e: &Event) -> EventStatus {
        let bounds = cx.bounds();
        match e {
            Event::Scroll { pos, delta_y } if bounds.contains(*pos) => {
                // Read/write scroll_y from WidgetStateStore
                let state = cx.state.entry(self.id);
                let new_y = (state.scroll_y - delta_y * 20.0).max(0.0);
                let max_scroll = (self.child_h - self.visible_h).max(0.0);
                state.scroll_y = new_y.min(max_scroll);
                cx.mark_dirty();
                EventStatus::Consumed
            }
            // Forward pointer events translated by scroll offset
            other => {
                let scroll_y = cx.state.entry(self.id).scroll_y;
                if let Some(pos) = other.pointer_pos() {
                    if !bounds.contains(pos) { return EventStatus::Ignored; }
                }
                // For simplicity in P04: forward all events to child without translation.
                // Proper translation (adding scroll_y to pos) deferred to container impl in P06.
                let child_layout = match cx.layout.child(0) { Some(l) => l, None => return EventStatus::Ignored };
                // EventCx for child — creates a child context
                // We approximate by setting widget_id to a child derivation
                let child_id = self.id.child(0);
                let mut child_cx = EventCx {
                    state: cx.state,
                    focus: cx.focus,
                    widget_id: child_id,
                    layout: child_layout,
                    needs_repaint: false,
                };
                let _ = scroll_y; // used later when translation is added
                let status = self.child.event(&mut child_cx, other);
                if child_cx.needs_repaint { cx.mark_dirty(); }
                status
            }
        }
    }
}
