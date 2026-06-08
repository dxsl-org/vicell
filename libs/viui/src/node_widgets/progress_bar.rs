// SPDX-License-Identifier: MIT
//! ProgressBar — display-only widget for sensor levels, battery, task completion.

extern crate alloc;
use alloc::vec::Vec;
use core::cell::Cell;

use crate::canvas::Color;
use crate::dirty::DirtyRegion;
use crate::event::Event;
use crate::layout::{Constraints, Point, Rect, Size};
use crate::node::ViNode;
use crate::render_ctx::RenderCtx;
use crate::signal::{Signal, SubscriptionHandle};

const BAR_H: f32 = 20.0;  // default horizontal bar height
const BAR_W: f32 = 20.0;  // default vertical bar width

/// Orientation of the progress bar.
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Orientation { Horizontal, Vertical }

/// Read-only progress indicator driven by a `Signal<f32>` in `[0.0, 1.0]`.
///
/// Values outside `[0.0, 1.0]` are clamped at paint time.
/// Subscribes to the signal so the app runner repaints on change.
pub struct ProgressBar {
    /// Value signal in `[0.0, 1.0]`.
    pub value:       Signal<f32>,
    /// Track (background) color.
    pub track_color: Color,
    /// Fill (foreground) color.
    pub fill_color:  Color,
    /// Bar orientation.
    pub orientation: Orientation,
    /// Show a percentage label on top of the bar.
    pub show_label:  bool,
    bounds:          Rect,
    _sub:            Cell<Option<SubscriptionHandle>>,
}

impl ProgressBar {
    pub fn new(value: Signal<f32>) -> Self {
        Self {
            value,
            track_color: Color::rgb(40, 40, 60),
            fill_color:  Color::rgb(60, 160, 60),
            orientation: Orientation::Horizontal,
            show_label:  false,
            bounds:      Rect::ZERO,
            _sub:        Cell::new(None),
        }
    }

    /// Set the fill color.
    pub fn color(mut self, fill: Color) -> Self { self.fill_color = fill; self }

    /// Set the track (background) color.
    pub fn track(mut self, track: Color) -> Self { self.track_color = track; self }

    /// Switch to vertical orientation.
    pub fn vertical(mut self) -> Self { self.orientation = Orientation::Vertical; self }

    /// Show a "XX%" label centered on the bar.
    pub fn with_label(mut self) -> Self { self.show_label = true; self }
}

impl ViNode for ProgressBar {
    fn layout(&mut self, constraints: Constraints) -> Size {
        let size = match self.orientation {
            Orientation::Horizontal => constraints.constrain(Size {
                w: constraints.max.w,
                h: BAR_H,
            }),
            Orientation::Vertical => constraints.constrain(Size {
                w: BAR_W,
                h: constraints.max.h,
            }),
        };
        self.bounds = Rect::from_origin_size(constraints.origin, size);
        size
    }

    fn bounds(&self) -> Rect { self.bounds }

    fn paint(&self, cx: &mut RenderCtx<'_>) {
        let v = self.value.get().clamp(0.0, 1.0);
        let b = self.bounds;

        // Track — use theme token, fall back to stored color if explicitly set
        cx.canvas.fill_rect(b, cx.theme.slider_track());

        // Fill — use theme token for the progress region
        let fill_color = cx.theme.progress_fill();
        let fill = match self.orientation {
            Orientation::Horizontal => Rect { x: b.x, y: b.y, w: b.w * v, h: b.h },
            Orientation::Vertical   => {
                let fh = b.h * v;
                Rect { x: b.x, y: b.y + b.h - fh, w: b.w, h: fh }
            }
        };
        if fill.w > 0.0 && fill.h > 0.0 {
            cx.canvas.fill_rect(fill, fill_color);
        }

        // Optional label: "XX%"
        if self.show_label {
            let pct = (v * 100.0) as u32;
            let label = format_pct(pct);
            let lx = b.x + b.w * 0.5 - label.len() as f32 * cx.char_width() * 0.5;
            let ly = b.y + b.h * 0.5 - cx.line_height() * 0.5;
            cx.draw_text(Point::new(lx.max(b.x), ly.max(b.y)), &label, Color::WHITE);
        }
    }

    fn event(&mut self, _event: &Event) -> bool { false }

    fn collect_dirty_handles(&mut self, region: DirtyRegion) -> Vec<SubscriptionHandle> {
        let rect = self.bounds;
        let h = self.value.subscribe(move || { region.borrow_mut().mark(rect); });
        alloc::vec![h]
    }
}

/// Format a percentage as a short string without std — e.g. 87 → "87%".
fn format_pct(pct: u32) -> alloc::string::String {
    use alloc::string::String;
    let mut s = String::new();
    if pct >= 100 {
        s.push_str("100%");
    } else {
        let tens = pct / 10;
        let ones = pct % 10;
        if tens > 0 { s.push((b'0' + tens as u8) as char); }
        s.push((b'0' + ones as u8) as char);
        s.push('%');
    }
    s
}
