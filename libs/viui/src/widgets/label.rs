//! Label widget — static text display.

use alloc::string::String;

use crate::canvas::{Color, TextStyle};
use crate::event::{Event, EventCx, EventStatus};
use crate::layout::{Constraints, LayoutNode, Rect, Size};
use crate::widget::{PaintCx, ViWidget};

const GLYPH_W: f32 = 8.0;
const GLYPH_H: f32 = 8.0;

pub struct Label {
    pub text:  String,
    pub style: TextStyle,
}

impl Label {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), style: TextStyle::DEFAULT }
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.style.color = color;
        self
    }

    /// Measure text width/height using the 8×8 bitmap font.
    pub fn measure(&self) -> Size {
        // Simple single-line measurement: no line wrapping.
        let chars = self.text.chars().count();
        Size { w: chars as f32 * GLYPH_W, h: GLYPH_H }
    }
}

impl ViWidget for Label {
    fn layout(&self, constraints: Constraints) -> LayoutNode {
        let desired = self.measure();
        let size = constraints.constrain(desired);
        let bounds = Rect::from_origin_size(constraints.origin, size);
        LayoutNode::leaf(bounds)
    }

    fn paint(&self, cx: &mut PaintCx) {
        let pos = cx.origin;
        cx.canvas.draw_text(pos, &self.text, self.style);
    }

    fn event(&mut self, _cx: &mut EventCx, _e: &Event) -> EventStatus {
        EventStatus::Ignored
    }
}
