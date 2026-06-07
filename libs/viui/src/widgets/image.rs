//! Image widget — static BGRA pixel blit.

use alloc::vec::Vec;

use crate::event::{Event, EventCx, EventStatus};
use crate::layout::{Constraints, LayoutNode, Rect, Size};
use crate::widget::{PaintCx, ViWidget};

pub struct Image {
    pixels: Vec<u8>,
    w:      u32,
    h:      u32,
}

impl Image {
    /// `pixels` must be BGRA8888 row-major, `w * h * 4` bytes total.
    pub fn new(pixels: Vec<u8>, w: u32, h: u32) -> Self {
        Self { pixels, w, h }
    }
}

impl ViWidget for Image {
    fn layout(&self, constraints: Constraints) -> LayoutNode {
        let desired = Size { w: self.w as f32, h: self.h as f32 };
        let size = constraints.constrain(desired);
        LayoutNode::leaf(Rect::from_origin_size(constraints.origin, size))
    }

    fn paint(&self, cx: &mut PaintCx) {
        let dest = Rect::from_origin_size(cx.origin, Size { w: self.w as f32, h: self.h as f32 });
        cx.canvas.draw_image(dest, &self.pixels, self.w * 4);
    }

    fn event(&mut self, _cx: &mut EventCx, _e: &Event) -> EventStatus {
        EventStatus::Ignored
    }
}
