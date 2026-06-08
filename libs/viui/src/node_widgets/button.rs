// SPDX-License-Identifier: MIT
//! Button v2 — clickable widget with a `Box<dyn Fn()>` callback.

extern crate alloc;
use alloc::{boxed::Box, string::String};

use crate::canvas::Color;
use crate::event::{Event, MouseButton};
use crate::layout::{Constraints, Point, Rect, Size};
use crate::node::ViNode;
use crate::render_ctx::RenderCtx;

const PAD: f32 = 6.0;

/// Clickable button.
///
/// `on_click` is called synchronously inside `event()` when a confirmed
/// left-click occurs (press + release on the button bounds).
pub struct Button {
    pub label:    String,
    pub on_click: Box<dyn Fn()>,
    hovered:      bool,
    pressed:      bool,
    bounds:       Rect,
}

impl Button {
    pub fn new(label: impl Into<String>, on_click: impl Fn() + 'static) -> Self {
        Self {
            label:    label.into(),
            on_click: Box::new(on_click),
            hovered:  false,
            pressed:  false,
            bounds:   Rect::ZERO,
        }
    }
}

impl ViNode for Button {
    fn layout(&mut self, constraints: Constraints) -> Size {
        let chars = self.label.chars().count();
        let desired = Size {
            w: chars as f32 * 8.0 + PAD * 2.0,
            h: 16.0 + PAD * 2.0,
        };
        let size = constraints.constrain(desired);
        self.bounds = Rect::from_origin_size(constraints.origin, size);
        size
    }

    fn bounds(&self) -> Rect { self.bounds }

    fn paint(&self, cx: &mut RenderCtx<'_>) {
        let bg = if self.pressed      { Color::rgb(80, 80, 160) }
                 else if self.hovered { Color::rgb(70, 70, 130) }
                 else                 { Color::rgb(50, 50, 100) };
        cx.canvas.fill_rect(self.bounds, bg);

        let b = self.bounds;
        let border = Color::rgb(120, 120, 200);
        for (a, bb) in [
            (Point::new(b.x,       b.y),       Point::new(b.x + b.w, b.y)),
            (Point::new(b.x + b.w, b.y),       Point::new(b.x + b.w, b.y + b.h)),
            (Point::new(b.x + b.w, b.y + b.h), Point::new(b.x,       b.y + b.h)),
            (Point::new(b.x,       b.y + b.h), Point::new(b.x,       b.y)),
        ] {
            cx.canvas.draw_line(a, bb, border);
        }

        cx.draw_text(Point::new(b.x + PAD, b.y + PAD), &self.label, Color::WHITE);
    }

    fn event(&mut self, event: &Event) -> bool {
        match event {
            Event::MouseMove { pos } => {
                self.hovered = self.bounds.contains(*pos);
                false
            }
            Event::MousePress { pos, button: MouseButton::Left } => {
                if self.bounds.contains(*pos) {
                    self.pressed = true;
                    true
                } else { false }
            }
            Event::MouseRelease { pos, button: MouseButton::Left } => {
                let was_pressed = self.pressed;
                self.pressed = false;
                if was_pressed && self.bounds.contains(*pos) {
                    (self.on_click)();
                    true
                } else { false }
            }
            Event::TouchBegin { pos, .. } => {
                if self.bounds.contains(*pos) {
                    self.pressed = true;
                    true
                } else { false }
            }
            Event::TouchEnd { pos, .. } => {
                let was_pressed = self.pressed;
                self.pressed = false;
                if was_pressed && self.bounds.contains(*pos) {
                    (self.on_click)();
                    true
                } else { false }
            }
            _ => false,
        }
    }
}
