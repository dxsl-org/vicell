//! Checkbox widget — toggle with label.

use alloc::string::String;

use crate::canvas::{Color, TextStyle};
use crate::event::{Event, EventCx, EventStatus, MouseButton};
use crate::layout::{Constraints, LayoutNode, Point, Rect, Size};
use crate::widget::{PaintCx, WidgetId, ViWidget};
use crate::widgets::label::Label;

const BOX_SIZE: f32 = 14.0;
const GAP:      f32 = 6.0;
const CHECK:    Color = Color::WHITE;

pub struct Checkbox {
    pub id:      WidgetId,
    pub checked: bool,
    label:       Label,
    hovered:     bool,
}

impl Checkbox {
    pub fn new(id: WidgetId, checked: bool, label: impl Into<String>) -> Self {
        Self { id, checked, label: Label::new(label), hovered: false }
    }

    /// Consume and return the latest checked state.
    pub fn is_checked(&self) -> bool { self.checked }
}

impl ViWidget for Checkbox {
    fn layout(&self, constraints: Constraints) -> LayoutNode {
        let label_w = self.label.measure().w;
        let desired = Size {
            w: BOX_SIZE + GAP + label_w,
            h: BOX_SIZE,
        };
        let size = constraints.constrain(desired);
        LayoutNode::leaf(Rect::from_origin_size(constraints.origin, size))
    }

    fn paint(&self, cx: &mut PaintCx) {
        let orig = cx.origin;
        let box_rect = Rect::new(orig.x, orig.y, BOX_SIZE, BOX_SIZE);

        let bg = if self.hovered { cx.theme.button_hovered() } else { cx.theme.surface() };
        cx.canvas.fill_rect(box_rect, bg);

        let border = cx.theme.border();
        for (a, b) in [
            (Point::new(orig.x, orig.y),            Point::new(orig.x + BOX_SIZE, orig.y)),
            (Point::new(orig.x + BOX_SIZE, orig.y), Point::new(orig.x + BOX_SIZE, orig.y + BOX_SIZE)),
            (Point::new(orig.x + BOX_SIZE, orig.y + BOX_SIZE), Point::new(orig.x, orig.y + BOX_SIZE)),
            (Point::new(orig.x, orig.y + BOX_SIZE), Point::new(orig.x, orig.y)),
        ] {
            cx.canvas.draw_line(a, b, border);
        }

        if self.checked {
            // Checkmark: two diagonal lines forming a tick
            let m = 3.0; // margin
            cx.canvas.draw_line(
                Point::new(orig.x + m, orig.y + BOX_SIZE / 2.0),
                Point::new(orig.x + BOX_SIZE / 2.0 - 1.0, orig.y + BOX_SIZE - m),
                CHECK,
            );
            cx.canvas.draw_line(
                Point::new(orig.x + BOX_SIZE / 2.0 - 1.0, orig.y + BOX_SIZE - m),
                Point::new(orig.x + BOX_SIZE - m, orig.y + m),
                CHECK,
            );
        }

        let text_pos = Point::new(orig.x + BOX_SIZE + GAP, orig.y + (BOX_SIZE - 8.0) / 2.0);
        cx.canvas.draw_text(text_pos, &self.label.text, TextStyle {
            color: cx.theme.text_primary(),
            size_px: cx.theme.font_size_body(),
        });
    }

    fn event(&mut self, cx: &mut EventCx, e: &Event) -> EventStatus {
        let bounds = cx.bounds();
        match e {
            Event::MouseMove { pos } => {
                let h = bounds.contains(*pos);
                if h != self.hovered { self.hovered = h; cx.mark_dirty(); }
                EventStatus::Ignored
            }
            Event::MousePress { pos, button: MouseButton::Left } => {
                if bounds.contains(*pos) {
                    self.checked = !self.checked;
                    cx.mark_dirty();
                    EventStatus::Consumed
                } else {
                    EventStatus::Ignored
                }
            }
            _ => EventStatus::Ignored,
        }
    }
}
