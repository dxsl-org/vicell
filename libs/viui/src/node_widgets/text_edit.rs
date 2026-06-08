// SPDX-License-Identifier: MIT
//! TextEdit — single-line text input with cursor and keyboard editing.

extern crate alloc;
use alloc::{boxed::Box, string::String, vec::Vec};
use core::cell::Cell;

use crate::canvas::Color;
use crate::dirty::DirtyRegion;
use crate::event::{Event, KeyCode};
use crate::layout::{Constraints, Point, Rect, Size};
use crate::node::ViNode;
use crate::render_ctx::RenderCtx;
use crate::signal::{Signal, SubscriptionHandle};

const HEIGHT:  f32 = 28.0;
const PADDING: f32 = 4.0;

/// Single-line text input field.
///
/// Receives keyboard events while focused (the event routing layer is expected
/// to dispatch `Focus`/`Blur` lifecycle events and `KeyPress`/`Char` direct
/// events to the focused widget).
///
/// # Cursor
///
/// Cursor position is a byte offset into the UTF-8 string. Navigation
/// (`Left`/`Right`/`Home`/`End`) advances by full Unicode scalar values.
pub struct TextEdit {
    /// Current text content.
    pub text:        Signal<String>,
    /// Hint shown when the field is empty and unfocused.
    pub placeholder: Signal<String>,
    on_submit:       Option<Box<dyn Fn(&str)>>,
    /// Byte offset of the cursor in `text`.
    cursor_pos:      Cell<usize>,
    focused:         Cell<bool>,
    bounds_cache:    Cell<Rect>,
}

impl TextEdit {
    pub fn new(text: Signal<String>) -> Self {
        Self {
            text,
            placeholder:  Signal::new(String::new()),
            on_submit:    None,
            cursor_pos:   Cell::new(0),
            focused:      Cell::new(false),
            bounds_cache: Cell::new(Rect::ZERO),
        }
    }

    /// Attach a reactive placeholder signal.
    pub fn placeholder(mut self, s: Signal<String>) -> Self {
        self.placeholder = s;
        self
    }

    /// Attach a static placeholder string.
    pub fn placeholder_str(self, s: impl Into<String>) -> Self {
        self.placeholder(Signal::new(s.into()))
    }

    /// Callback fired with the current text when the user presses Enter.
    pub fn on_submit(mut self, f: impl Fn(&str) + 'static) -> Self {
        self.on_submit = Some(Box::new(f));
        self
    }

    /// Advance `pos` by one Unicode scalar in `text`.
    fn advance_cursor(text: &str, pos: usize) -> usize {
        text[pos..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| pos + i)
            .unwrap_or(text.len())
    }

    /// Retreat `pos` by one Unicode scalar in `text`.
    fn retreat_cursor(text: &str, pos: usize) -> usize {
        text[..pos]
            .char_indices()
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0)
    }
}

impl ViNode for TextEdit {
    fn layout(&mut self, constraints: Constraints) -> Size {
        let size = constraints.constrain(Size { w: constraints.max.w, h: HEIGHT });
        self.bounds_cache.set(Rect::from_origin_size(constraints.origin, size));
        size
    }

    fn bounds(&self) -> Rect { self.bounds_cache.get() }

    fn paint(&self, cx: &mut RenderCtx<'_>) {
        let b = self.bounds_cache.get();

        // Background — slightly brighter when focused
        let bg = if self.focused.get() {
            Color::rgb(20, 20, 50)
        } else {
            Color::rgb(25, 25, 38)
        };
        cx.canvas.fill_rect(b, bg);

        // Border — accent color when focused
        let border = if self.focused.get() {
            Color::rgb(80, 120, 220)
        } else {
            Color::rgb(70, 70, 95)
        };
        let x0 = b.x;
        let y0 = b.y;
        let x1 = b.x + b.w;
        let y1 = b.y + b.h;
        cx.canvas.draw_line(Point::new(x0, y0), Point::new(x1, y0), border); // top
        cx.canvas.draw_line(Point::new(x1, y0), Point::new(x1, y1), border); // right
        cx.canvas.draw_line(Point::new(x1, y1), Point::new(x0, y1), border); // bottom
        cx.canvas.draw_line(Point::new(x0, y1), Point::new(x0, y0), border); // left

        let ty = b.y + (b.h - cx.line_height()) * 0.5;

        // Clip text content to the inner area
        cx.canvas.clip_push(Rect { x: b.x + 1.0, y: b.y + 1.0, w: b.w - 2.0, h: b.h - 2.0 });

        let text = self.text.get();
        if text.is_empty() && !self.focused.get() {
            // Show placeholder
            let ph = self.placeholder.get();
            if !ph.is_empty() {
                cx.draw_text(
                    Point::new(b.x + PADDING, ty.max(b.y)),
                    &ph,
                    Color::rgb(100, 100, 130),
                );
            }
        } else {
            // Draw the text content
            cx.draw_text(
                Point::new(b.x + PADDING, ty.max(b.y)),
                &text,
                Color::rgb(220, 220, 230),
            );

            // Draw cursor bar while focused
            if self.focused.get() {
                let cursor_byte = self.cursor_pos.get().min(text.len());
                let char_count  = text[..cursor_byte].chars().count() as f32;
                let cursor_x    = b.x + PADDING + char_count * cx.char_width();
                cx.canvas.draw_line(
                    Point::new(cursor_x, b.y + 3.0),
                    Point::new(cursor_x, b.y + b.h - 3.0),
                    Color::WHITE,
                );
            }
        }

        cx.canvas.clip_pop();
    }

    fn event(&mut self, event: &Event) -> bool {
        match event {
            Event::Focus => { self.focused.set(true); true }
            Event::Blur  => { self.focused.set(false); true }

            // Character input — insert at cursor
            Event::Char(ch) if self.focused.get() => {
                let pos = self.cursor_pos.get();
                let byte_len = ch.len_utf8();
                self.text.update(|s| { s.insert(pos, *ch); });
                self.cursor_pos.set(pos + byte_len);
                true
            }

            // Structural keys
            Event::KeyPress { key, .. } if self.focused.get() => {
                match key {
                    KeyCode::Backspace => {
                        let pos = self.cursor_pos.get();
                        if pos > 0 {
                            // Need a snapshot for retreat calculation before mutating
                            let new_pos = {
                                let text = self.text.get();
                                Self::retreat_cursor(&text, pos)
                            };
                            self.text.update(|s| { s.remove(new_pos); });
                            self.cursor_pos.set(new_pos);
                        }
                        true
                    }
                    KeyCode::Delete => {
                        let pos = self.cursor_pos.get();
                        let len = self.text.get().len();
                        if pos < len {
                            self.text.update(|s| { s.remove(pos); });
                        }
                        true
                    }
                    KeyCode::Left => {
                        let pos = self.cursor_pos.get();
                        let new_pos = {
                            let text = self.text.get();
                            Self::retreat_cursor(&text, pos)
                        };
                        self.cursor_pos.set(new_pos);
                        true
                    }
                    KeyCode::Right => {
                        let pos = self.cursor_pos.get();
                        let new_pos = {
                            let text = self.text.get();
                            Self::advance_cursor(&text, pos)
                        };
                        self.cursor_pos.set(new_pos);
                        true
                    }
                    KeyCode::Home => { self.cursor_pos.set(0); true }
                    KeyCode::End  => {
                        let len = self.text.get().len();
                        self.cursor_pos.set(len);
                        true
                    }
                    KeyCode::Enter => {
                        // Clone text before calling callback to release the Ref
                        let text_snap: String = (*self.text.get()).clone();
                        if let Some(cb) = &self.on_submit { cb(&text_snap); }
                        true
                    }
                    _ => false,
                }
            }

            // Gain focus on click
            Event::MousePress { pos, .. } if self.bounds_cache.get().contains(*pos) => {
                self.focused.set(true);
                true
            }

            _ => false,
        }
    }

    fn collect_dirty_handles(&mut self, region: DirtyRegion) -> Vec<SubscriptionHandle> {
        let bounds = self.bounds_cache.get();
        let h = self.text.subscribe(move || { region.borrow_mut().mark(bounds); });
        alloc::vec![h]
    }
}
