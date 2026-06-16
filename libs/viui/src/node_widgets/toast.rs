// SPDX-License-Identifier: MIT
//! Toast notification widget — transient, auto-dismiss, non-blocking overlay.
//!
//! Toasts are positioned at the bottom-center of the screen. Auto-dismiss is
//! driven by `ToastEntry::elapsed_ms`, which is incremented by `ViApp::tick_with_dt`.
//! Set `duration_ms = 0` for a manual-dismiss-only toast.

extern crate alloc;
use alloc::string::String;
use core::cell::Cell;

use crate::canvas::Color;
use crate::dirty::DirtyRegion;
use crate::event::Event;
use crate::layout::{Constraints, Point, Rect, Size};
use crate::node::ViNode;
use crate::render_ctx::RenderCtx;
use crate::signal::SubscriptionHandle;

// ─── ToastKind ───────────────────────────────────────────────────────────────

/// Semantic category of a toast — controls background colour.
#[derive(Clone)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

// ─── ToastConfig ─────────────────────────────────────────────────────────────

/// Parameters for a transient toast notification.
#[derive(Clone)]
pub struct ToastConfig {
    pub message:     String,
    pub kind:        ToastKind,
    /// Auto-dismiss after this many milliseconds. `0` = manual dismiss only.
    pub duration_ms: u32,
}

// ─── ToastEntry ──────────────────────────────────────────────────────────────

/// Live toast tracked by `ViApp` — wraps the config with timer state and a
/// pre-built widget so `paint()` does not need to allocate each frame.
pub struct ToastEntry {
    pub config:     ToastConfig,
    pub elapsed_ms: u32,
    /// Pre-built widget. Laid out every frame by `ViApp` in Path A.
    pub widget:     Toast,
}

// ─── Toast ───────────────────────────────────────────────────────────────────

/// Rendered toast overlay widget.
///
/// Positioned at the bottom-center of the screen constraints. The widget is
/// non-interactive: all events pass through.
pub struct Toast {
    message: String,
    kind:    ToastKind,
    bounds:  Cell<Rect>,
}

impl Toast {
    /// Construct from a `ToastConfig`.
    pub fn new(config: &ToastConfig) -> Self {
        Self {
            message: config.message.clone(),
            kind:    config.kind.clone(),
            bounds:  Cell::new(Rect::ZERO),
        }
    }

    fn bg_color(&self) -> Color {
        match self.kind {
            ToastKind::Info    => Color::rgb(40, 80, 160),
            ToastKind::Success => Color::rgb(30, 120, 60),
            ToastKind::Warning => Color::rgb(160, 120, 20),
            ToastKind::Error   => Color::rgb(160, 40, 40),
        }
    }
}

impl ViNode for Toast {
    fn layout(&mut self, constraints: Constraints) -> Size {
        let w = constraints.max.w.min(400.0);
        let h = 44.0f32;
        // Centre horizontally; sit 16 px above the bottom edge.
        let x = constraints.origin.x + (constraints.max.w - w) / 2.0;
        let y = constraints.origin.y + constraints.max.h - h - 16.0;
        self.bounds.set(Rect { x, y, w, h });
        Size::new(w, h)
    }

    fn bounds(&self) -> Rect { self.bounds.get() }

    fn paint(&self, cx: &mut RenderCtx<'_>) {
        let b = self.bounds.get();
        cx.canvas.fill_rect(b, self.bg_color());
        // 12 px left pad, vertical centre of 44 px = ~14 px from top (8 px glyph).
        cx.draw_text(Point::new(b.x + 12.0, b.y + 14.0), &self.message, Color::WHITE);
    }

    /// Toasts are non-interactive — never consume events.
    fn event(&mut self, _event: &Event) -> bool { false }

    fn collect_dirty_handles(&mut self, _region: DirtyRegion) -> alloc::vec::Vec<SubscriptionHandle> {
        alloc::vec::Vec::new()
    }
}
