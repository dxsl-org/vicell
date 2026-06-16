// SPDX-License-Identifier: MIT
//! Dialog overlay widget — blocking modal card with title, message, and buttons.
//!
//! Construct via `Dialog::alert` (single OK button) or `Dialog::confirm`
//! (Cancel + Confirm). Both constructors accept an `OverlayActionQueue`; button
//! callbacks push `OverlayAction::Pop` to close the dialog after the user action.
//!
//! # Event handling
//!
//! Because Dialog is pushed as a **blocking** overlay, `ViApp` does not forward
//! events to the root widget while the dialog is visible. The dialog itself
//! consumes all events; clicks outside the dialog bounds also pop it.

extern crate alloc;
use alloc::{boxed::Box, string::String, vec::Vec};
use core::cell::Cell;

use crate::canvas::Color;
use crate::dirty::DirtyRegion;
use crate::event::{Event, MouseButton};
use crate::layout::{Constraints, Point, Rect, Size};
use crate::node::ViNode;
use crate::overlay::{OverlayAction, OverlayActionQueue};
use crate::render_ctx::RenderCtx;
use crate::signal::SubscriptionHandle;

// ─── DialogButton ────────────────────────────────────────────────────────────

struct DialogButton {
    label:      String,
    is_primary: bool,
    /// Called on confirmed click (MouseRelease inside bounds after MousePress).
    action:     Box<dyn Fn()>,
    bounds:     Cell<Rect>,
    hovered:    bool,
    pressed:    bool,
}

// ─── Dialog ──────────────────────────────────────────────────────────────────

/// Blocking modal dialog.
///
/// Rendered as a centred card over a translucent scrim. The dialog lays itself
/// out during `ViApp`'s layout pass, so its bounds are always screen-centred
/// regardless of the window size.
pub struct Dialog {
    title:   String,
    message: String,
    buttons: Vec<DialogButton>,
    bounds:  Cell<Rect>,
    /// Used by `event()` to push `Pop` after a button action executes.
    queue:   OverlayActionQueue,
}

impl Dialog {
    /// Simple alert dialog with a single OK button.
    ///
    /// `on_ok` is called before the dialog pops itself.
    pub fn alert(
        title:   impl Into<String>,
        message: impl Into<String>,
        queue:   OverlayActionQueue,
        on_ok:   impl Fn() + 'static,
    ) -> Self {
        let q = queue.clone();
        Self {
            title:   title.into(),
            message: message.into(),
            buttons: alloc::vec![DialogButton {
                label:      "OK".into(),
                is_primary: true,
                action:     Box::new(move || {
                    on_ok();
                    q.borrow_mut().push(OverlayAction::Pop);
                }),
                bounds:  Cell::new(Rect::ZERO),
                hovered: false,
                pressed: false,
            }],
            bounds: Cell::new(Rect::ZERO),
            queue,
        }
    }

    /// Confirm dialog with Cancel (secondary) and Confirm (primary) buttons.
    ///
    /// `on_confirm` / `on_cancel` are called before the dialog pops itself.
    pub fn confirm(
        title:      impl Into<String>,
        message:    impl Into<String>,
        queue:      OverlayActionQueue,
        on_confirm: impl Fn() + 'static,
        on_cancel:  impl Fn() + 'static,
    ) -> Self {
        let q1 = queue.clone();
        let q2 = queue.clone();
        Self {
            title:   title.into(),
            message: message.into(),
            buttons: alloc::vec![
                DialogButton {
                    label:      "Cancel".into(),
                    is_primary: false,
                    action:     Box::new(move || {
                        on_cancel();
                        q1.borrow_mut().push(OverlayAction::Pop);
                    }),
                    bounds:  Cell::new(Rect::ZERO),
                    hovered: false,
                    pressed: false,
                },
                DialogButton {
                    label:      "Confirm".into(),
                    is_primary: true,
                    action:     Box::new(move || {
                        on_confirm();
                        q2.borrow_mut().push(OverlayAction::Pop);
                    }),
                    bounds:  Cell::new(Rect::ZERO),
                    hovered: false,
                    pressed: false,
                },
            ],
            bounds: Cell::new(Rect::ZERO),
            queue,
        }
    }
}

impl ViNode for Dialog {
    fn layout(&mut self, constraints: Constraints) -> Size {
        // Card: 320×160 px centred on screen, clamped to available space.
        let w = 320.0f32.min(constraints.max.w - 32.0).max(120.0);
        let h = 160.0f32;
        let x = constraints.origin.x + (constraints.max.w - w) / 2.0;
        let y = constraints.origin.y + (constraints.max.h - h) / 2.0;
        self.bounds.set(Rect { x, y, w, h });

        // Buttons: right-aligned row at bottom of card (12 px margin).
        let btn_w = 80.0f32;
        let btn_h = 32.0f32;
        let btn_y = y + h - btn_h - 12.0;
        let n = self.buttons.len() as f32;
        for (i, btn) in self.buttons.iter_mut().enumerate() {
            // Stack right-to-left: primary button is rightmost.
            let slot = (n - 1.0 - i as f32);
            let btn_x = x + w - 12.0 - (slot + 1.0) * (btn_w + 8.0) + 8.0;
            btn.bounds.set(Rect { x: btn_x, y: btn_y, w: btn_w, h: btn_h });
        }

        Size::new(w, h)
    }

    fn bounds(&self) -> Rect { self.bounds.get() }

    fn paint(&self, cx: &mut RenderCtx<'_>) {
        let b = self.bounds.get();

        // Card background.
        cx.canvas.fill_rect(b, Color::rgb(30, 32, 44));
        // Subtle border.
        cx.canvas.draw_rect_border(b, Color::rgb(70, 72, 100), 1.0);

        // Title text.
        cx.draw_text(Point::new(b.x + 16.0, b.y + 16.0), &self.title, Color::WHITE);
        // Message text — slightly dimmed.
        cx.draw_text(Point::new(b.x + 16.0, b.y + 48.0), &self.message, Color::rgb(180, 180, 200));

        // Buttons.
        for btn in &self.buttons {
            let bb = btn.bounds.get();
            let bg = if btn.is_primary {
                if btn.pressed      { Color::rgb(60, 100, 200) }
                else if btn.hovered { Color::rgb(100, 140, 240) }
                else                { Color::rgb(80, 120, 220) }
            } else {
                if btn.pressed      { Color::rgb(50, 52, 70) }
                else if btn.hovered { Color::rgb(70, 72, 90) }
                else                { Color::rgb(55, 57, 75) }
            };
            cx.canvas.fill_rect(bb, bg);

            // Centre label in button (approx: 8 px/char, 8 px glyph height).
            let chars  = btn.label.chars().count() as f32;
            let text_x = bb.x + (bb.w - chars * 8.0) / 2.0;
            let text_y = bb.y + (bb.h - 8.0) / 2.0;
            cx.draw_text(Point::new(text_x, text_y), &btn.label, Color::WHITE);
        }
    }

    /// Blocking dialog: consume all events.
    ///
    /// Clicks outside the card dismiss the dialog. Button presses/releases
    /// are tracked to give visual feedback and trigger the action on release.
    fn event(&mut self, event: &Event) -> bool {
        match event {
            Event::MouseMove { pos } => {
                for btn in &mut self.buttons {
                    btn.hovered = btn.bounds.get().contains(*pos);
                }
                // Consume: prevent the root from acting on mouse moves over the dim.
                true
            }
            Event::MousePress { pos, button: MouseButton::Left } => {
                if self.bounds.get().contains(*pos) {
                    for btn in &mut self.buttons {
                        if btn.bounds.get().contains(*pos) {
                            btn.pressed = true;
                        }
                    }
                } else {
                    // Click outside → dismiss.
                    self.queue.borrow_mut().push(OverlayAction::Pop);
                }
                true // always consume
            }
            Event::MouseRelease { pos, button: MouseButton::Left } => {
                for btn in &mut self.buttons {
                    let was_pressed = btn.pressed;
                    btn.pressed = false;
                    if was_pressed && btn.bounds.get().contains(*pos) {
                        (btn.action)();
                        return true;
                    }
                }
                true
            }
            // Blocking overlay: consume everything so nothing leaks to root.
            _ => true,
        }
    }

    fn collect_dirty_handles(&mut self, _region: DirtyRegion) -> Vec<SubscriptionHandle> {
        Vec::new()
    }

    fn is_focusable(&self) -> bool { false }
}
