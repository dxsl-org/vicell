//! Space widget — invisible spacer with fixed dimensions.

use crate::event::{Event, EventCx, EventStatus};
use crate::layout::{Constraints, LayoutNode, Rect, Size};
use crate::widget::{PaintCx, ViWidget};

pub struct Space { pub w: f32, pub h: f32 }

impl Space {
    pub fn new(w: f32, h: f32) -> Self { Self { w, h } }
    pub fn vertical(h: f32)   -> Self { Self { w: 0.0, h } }
    pub fn horizontal(w: f32) -> Self { Self { w, h: 0.0 } }
}

impl ViWidget for Space {
    fn layout(&self, constraints: Constraints) -> LayoutNode {
        let size = constraints.constrain(Size { w: self.w, h: self.h });
        LayoutNode::leaf(Rect::from_origin_size(constraints.origin, size))
    }
    fn paint(&self, _cx: &mut PaintCx) {}
    fn event(&mut self, _cx: &mut EventCx, _e: &Event) -> EventStatus { EventStatus::Ignored }
}
