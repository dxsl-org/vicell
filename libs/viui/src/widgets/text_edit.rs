//! TextEdit widget — single-line text input with keyboard editing.

use alloc::string::String;

use crate::canvas::{Color, TextStyle};  // Color used for CURSOR_CLR constant
use crate::event::{Event, EventCx, EventStatus, KeyCode, MouseButton};
use crate::layout::{Constraints, LayoutNode, Length, Point, Rect, Size};
use crate::widget::{PaintCx, WidgetId, ViWidget};

const GLYPH_W: f32 = 8.0;
const GLYPH_H: f32 = 8.0;
const PAD:     f32 = 4.0;
const HEIGHT:  f32 = GLYPH_H + PAD * 2.0;

const CURSOR_CLR: Color = Color::WHITE;

pub struct TextEdit {
    pub id:      WidgetId,
    pub text:    String,
    cursor:      usize,   // char index (NOT byte offset)
    focused:     bool,
    width:       Length,
}

impl TextEdit {
    pub fn new(id: WidgetId) -> Self {
        Self { id, text: String::new(), cursor: 0, focused: false, width: Length::Fill }
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self.cursor = self.text.chars().count();
        self
    }

    pub fn with_width(mut self, w: Length) -> Self {
        self.width = w;
        self
    }

    fn char_count(&self) -> usize { self.text.chars().count() }

    fn insert_char(&mut self, ch: char) {
        // Convert char index to byte offset
        let byte_off = self.text.char_indices()
            .nth(self.cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len());
        self.text.insert(byte_off, ch);
        self.cursor += 1;
    }

    fn delete_before_cursor(&mut self) {
        if self.cursor == 0 { return; }
        let byte_off = self.text.char_indices()
            .nth(self.cursor - 1)
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.text.remove(byte_off);
        self.cursor -= 1;
    }

    fn delete_at_cursor(&mut self) {
        if self.cursor >= self.char_count() { return; }
        let byte_off = self.text.char_indices()
            .nth(self.cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len());
        self.text.remove(byte_off);
    }
}

impl ViWidget for TextEdit {
    fn layout(&self, constraints: Constraints) -> LayoutNode {
        let w = match self.width {
            Length::Fill | Length::FillPortion(_) => constraints.max.w,
            Length::Fixed(v) => v,
            Length::Shrink => {
                let chars = self.text.chars().count();
                ((chars + 2) as f32 * GLYPH_W + PAD * 2.0).max(40.0)
            }
        };
        let size = constraints.constrain(Size { w, h: HEIGHT });
        LayoutNode::leaf(Rect::from_origin_size(constraints.origin, size))
    }

    fn paint(&self, cx: &mut PaintCx) {
        let bounds = Rect::new(cx.origin.x, cx.origin.y, cx.canvas.width() as f32, HEIGHT);
        let bg = if self.focused { cx.theme.input_focused_bg() } else { cx.theme.input_bg() };
        cx.canvas.fill_rect(bounds, bg);

        let border = if self.focused { cx.theme.input_focused_border() } else { cx.theme.border() };
        cx.canvas.draw_line(
            Point::new(bounds.x, bounds.y + bounds.h - 1.0),
            Point::new(bounds.x + bounds.w, bounds.y + bounds.h - 1.0),
            border,
        );

        let text_pos = Point::new(bounds.x + PAD, bounds.y + PAD);
        cx.canvas.draw_text(text_pos, &self.text, TextStyle { color: cx.theme.text_primary(), size_px: 0 });

        // Cursor
        if self.focused {
            let cx_x = bounds.x + PAD + self.cursor as f32 * GLYPH_W;
            let cy0 = bounds.y + PAD;
            let cy1 = bounds.y + PAD + GLYPH_H;
            cx.canvas.draw_line(Point::new(cx_x, cy0), Point::new(cx_x, cy1), CURSOR_CLR);
        }
    }

    fn event(&mut self, cx: &mut EventCx, e: &Event) -> EventStatus {
        let bounds = cx.bounds();
        match e {
            Event::MousePress { pos, button: MouseButton::Left } => {
                if bounds.contains(*pos) {
                    if !self.focused {
                        self.focused = true;
                        cx.set_focus(self.id);
                        cx.mark_dirty();
                    }
                    // Move cursor to click position
                    let rel_x = (pos.x - bounds.x - PAD).max(0.0);
                    self.cursor = ((rel_x / GLYPH_W) as usize).min(self.char_count());
                    cx.mark_dirty();
                    EventStatus::Consumed
                } else {
                    if self.focused {
                        self.focused = false;
                        cx.release_focus();
                        cx.mark_dirty();
                    }
                    EventStatus::Ignored
                }
            }
            Event::Char(ch) if self.focused => {
                if !ch.is_control() {
                    self.insert_char(*ch);
                    cx.mark_dirty();
                    EventStatus::Consumed
                } else {
                    EventStatus::Ignored
                }
            }
            Event::KeyPress { key, .. } if self.focused => {
                match key {
                    KeyCode::Backspace => { self.delete_before_cursor(); cx.mark_dirty(); }
                    KeyCode::Delete    => { self.delete_at_cursor();     cx.mark_dirty(); }
                    KeyCode::Left  => { if self.cursor > 0 { self.cursor -= 1; cx.mark_dirty(); } }
                    KeyCode::Right => {
                        if self.cursor < self.char_count() { self.cursor += 1; cx.mark_dirty(); }
                    }
                    KeyCode::Home => { self.cursor = 0; cx.mark_dirty(); }
                    KeyCode::End  => { self.cursor = self.char_count(); cx.mark_dirty(); }
                    KeyCode::Escape => {
                        self.focused = false;
                        cx.release_focus();
                        cx.mark_dirty();
                    }
                    _ => return EventStatus::Ignored,
                }
                EventStatus::Consumed
            }
            Event::Blur => {
                self.focused = false;
                cx.mark_dirty();
                EventStatus::Ignored
            }
            _ => EventStatus::Ignored,
        }
    }
}
