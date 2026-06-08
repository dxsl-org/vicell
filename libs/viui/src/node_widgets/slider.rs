// SPDX-License-Identifier: MIT
//! Slider — horizontal drag widget for parameter tuning (speed, gain, threshold).

extern crate alloc;
use alloc::{boxed::Box, vec};
use alloc::vec::Vec;
use core::cell::Cell;

use crate::dirty::DirtyRegion;
use crate::event::{Event, MouseButton};
use crate::layout::{Constraints, Point, Rect, Size};
use crate::node::ViNode;
use crate::render_ctx::RenderCtx;
use crate::signal::{Signal, SubscriptionHandle};

const HEIGHT:      f32 = 24.0;  // total widget height (track + thumb headroom)
const TRACK_H:     f32 = 4.0;   // track bar height in pixels
const THUMB_R:     f32 = 8.0;   // thumb circle radius

/// Horizontal slider — value in `[0.0, 1.0]`.
///
/// The signal is readable from outside; on_change fires whenever the user drags.
/// Supports both mouse and touch input.
pub struct Slider {
    /// Current normalized value `[0.0, 1.0]`.
    pub value:     Signal<f32>,
    on_change:     Option<Box<dyn Fn(f32)>>,
    dragging:      Cell<bool>,
    /// Bounds cached from the last `layout()` call, used in `event()`.
    bounds_cache:  Cell<Rect>,
}

impl Slider {
    pub fn new(value: Signal<f32>) -> Self {
        Self {
            value,
            on_change:    None,
            dragging:     Cell::new(false),
            bounds_cache: Cell::new(Rect::ZERO),
        }
    }

    /// Register a callback fired with the new value on every drag update.
    pub fn on_change(mut self, f: impl Fn(f32) + 'static) -> Self {
        self.on_change = Some(Box::new(f));
        self
    }
}

impl ViNode for Slider {
    fn layout(&mut self, constraints: Constraints) -> Size {
        let size = constraints.constrain(Size { w: constraints.max.w, h: HEIGHT });
        let bounds = Rect::from_origin_size(constraints.origin, size);
        self.bounds_cache.set(bounds);
        size
    }

    fn bounds(&self) -> Rect { self.bounds_cache.get() }

    fn paint(&self, cx: &mut RenderCtx<'_>) {
        let b      = self.bounds_cache.get();
        let cy     = b.y + b.h * 0.5;
        let track  = Rect { x: b.x + THUMB_R, y: cy - TRACK_H * 0.5, w: b.w - THUMB_R * 2.0, h: TRACK_H };

        // Track
        cx.canvas.fill_rect(track, cx.theme.slider_track());

        // Filled portion
        let v = self.value.get().clamp(0.0, 1.0);
        let filled = Rect { x: track.x, y: track.y, w: track.w * v, h: track.h };
        if filled.w > 0.0 {
            cx.canvas.fill_rect(filled, cx.theme.accent());
        }

        // Thumb — drawn as a small square (circle approximation, no trig needed)
        let tx = b.x + THUMB_R + (b.w - THUMB_R * 2.0) * v - THUMB_R;
        let ty = cy - THUMB_R;
        let thumb = Rect { x: tx, y: ty, w: THUMB_R * 2.0, h: THUMB_R * 2.0 };
        let thumb_color = if self.dragging.get() { cx.theme.button_pressed() } else { cx.theme.accent() };
        cx.canvas.fill_rect(thumb, thumb_color);
        // Thin border around thumb
        let border = cx.theme.border();
        for (a, bb) in [
            (Point::new(thumb.x,            thumb.y),            Point::new(thumb.x + thumb.w, thumb.y)),
            (Point::new(thumb.x + thumb.w,  thumb.y),            Point::new(thumb.x + thumb.w, thumb.y + thumb.h)),
            (Point::new(thumb.x + thumb.w,  thumb.y + thumb.h),  Point::new(thumb.x,           thumb.y + thumb.h)),
            (Point::new(thumb.x,            thumb.y + thumb.h),  Point::new(thumb.x,           thumb.y)),
        ] {
            cx.canvas.draw_line(a, bb, border);
        }
    }

    fn event(&mut self, event: &Event) -> bool {
        let b = self.bounds_cache.get();
        let track_x   = b.x + THUMB_R;
        let track_w   = (b.w - THUMB_R * 2.0).max(1.0);

        let set_from_pos = |pos: Point| -> bool {
            let v = ((pos.x - track_x) / track_w).clamp(0.0, 1.0);
            self.value.set(v);
            if let Some(cb) = &self.on_change { cb(v); }
            true
        };

        match event {
            Event::MousePress { pos, button: MouseButton::Left } if b.contains(*pos) => {
                self.dragging.set(true);
                set_from_pos(*pos)
            }
            Event::MouseMove { pos } if self.dragging.get() => {
                set_from_pos(*pos)
            }
            Event::MouseRelease { .. } => {
                self.dragging.set(false);
                false
            }
            Event::TouchBegin { pos, finger_id: 0 } if b.contains(*pos) => {
                self.dragging.set(true);
                set_from_pos(*pos)
            }
            Event::TouchMove { pos, finger_id: 0 } if self.dragging.get() => {
                set_from_pos(*pos)
            }
            Event::TouchEnd { finger_id: 0, .. } => {
                self.dragging.set(false);
                false
            }
            _ => false,
        }
    }

    fn collect_dirty_handles(&mut self, region: DirtyRegion) -> Vec<SubscriptionHandle> {
        let bounds = self.bounds_cache.get();
        let h = self.value.subscribe(move || { region.borrow_mut().mark(bounds); });
        vec![h]
    }
}
