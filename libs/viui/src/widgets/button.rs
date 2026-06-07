//! Button widget — OrbTK WidgetFlags + decoupled event→message pattern.

use alloc::string::String;

use crate::canvas::TextStyle;
use crate::event::{Event, EventCx, EventStatus, MouseButton};
use crate::layout::{Constraints, LayoutNode, Padding, Point, Rect, Size};
use crate::widget::{PaintCx, WidgetId, ViWidget};
use crate::widgets::label::Label;

pub struct Button {
    pub id:      WidgetId,
    label:       Label,
    pub padding: Padding,
    // Direct-field state (OrbTK pattern: event() writes, paint() reads)
    hovered:     bool,
    pressed:     bool,
    /// True for exactly one frame after a completed click. Reset each dispatch cycle.
    pub just_clicked: bool,
}

impl Button {
    pub fn new(id: WidgetId, text: impl Into<String>) -> Self {
        Self {
            id,
            label: Label::new(text),
            padding: Padding::all(6.0),
            hovered: false,
            pressed: false,
            just_clicked: false,
        }
    }

    pub fn with_padding(mut self, padding: Padding) -> Self {
        self.padding = padding;
        self
    }

    /// Call once per frame after dispatch to consume and reset the click flag.
    pub fn clicked(&mut self) -> bool {
        let v = self.just_clicked;
        self.just_clicked = false;
        v
    }
}

impl ViWidget for Button {
    fn layout(&self, constraints: Constraints) -> LayoutNode {
        let text_size = self.label.measure();
        let desired = Size {
            w: text_size.w + self.padding.h_total(),
            h: text_size.h + self.padding.v_total(),
        };
        let size = constraints.constrain(desired);
        let bounds = Rect::from_origin_size(constraints.origin, size);
        LayoutNode::leaf(bounds)
    }

    fn paint(&self, cx: &mut PaintCx) {
        let ts = self.label.measure();
        let bounds = Rect::from_origin_size(cx.origin, Size {
            w: ts.w + self.padding.h_total(),
            h: ts.h + self.padding.v_total(),
        });

        let bg = if self.pressed { cx.theme.button_pressed() }
                 else if self.hovered { cx.theme.button_hovered() }
                 else { cx.theme.button_normal() };
        cx.canvas.fill_rect(bounds, bg);

        let border = cx.theme.border();
        let b = bounds;
        for (a, bb) in [
            (Point::new(b.x, b.y),         Point::new(b.x + b.w, b.y)),
            (Point::new(b.x + b.w, b.y),   Point::new(b.x + b.w, b.y + b.h)),
            (Point::new(b.x + b.w, b.y + b.h), Point::new(b.x, b.y + b.h)),
            (Point::new(b.x, b.y + b.h),   Point::new(b.x, b.y)),
        ] {
            cx.canvas.draw_line(a, bb, border);
        }

        let text_pos = Point::new(cx.origin.x + self.padding.left, cx.origin.y + self.padding.top);
        cx.canvas.draw_text(text_pos, &self.label.text, TextStyle {
            color: cx.theme.text_primary(),
            size_px: cx.theme.font_size_body(),
        });
    }

    fn event(&mut self, cx: &mut EventCx, e: &Event) -> EventStatus {
        let bounds = cx.bounds();
        match e {
            Event::MouseMove { pos } => {
                let now_hovered = bounds.contains(*pos);
                if now_hovered != self.hovered {
                    self.hovered = now_hovered;
                    cx.mark_dirty();
                }
                EventStatus::Ignored
            }
            Event::MousePress { pos, button: MouseButton::Left } => {
                if bounds.contains(*pos) {
                    self.pressed = true;
                    cx.mark_dirty();
                    EventStatus::Consumed
                } else {
                    EventStatus::Ignored
                }
            }
            Event::MouseRelease { pos, button: MouseButton::Left } => {
                let was_pressed = self.pressed;
                self.pressed = false;
                if was_pressed {
                    cx.mark_dirty();
                    if bounds.contains(*pos) {
                        self.just_clicked = true;
                    }
                    EventStatus::Consumed
                } else {
                    EventStatus::Ignored
                }
            }
            _ => EventStatus::Ignored,
        }
    }
}
