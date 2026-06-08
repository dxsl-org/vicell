// SPDX-License-Identifier: MIT
//! Dirty-rectangle accumulator for ViUI v2 partial repaint.
//!
//! Each widget marks its bounds dirty when a Signal it depends on changes.
//! The renderer consumes the accumulated rect via `take()` and repaints only
//! that region, skipping the rest of the surface entirely.

extern crate alloc;
use alloc::rc::Rc;
use core::cell::RefCell;
use crate::layout::Rect;

/// Shared handle to a `DirtyRect` — passed to widget Signal subscriptions.
///
/// Widgets subscribe their Signals to call `region.borrow_mut().mark(bounds)`
/// when their data changes, queuing a partial repaint without a full layout pass.
pub type DirtyRegion = Rc<RefCell<DirtyRect>>;

/// Accumulates damaged screen regions into a single union rect per frame.
///
/// # Usage
///
/// 1. Wire: `signal.subscribe(|| dirty.mark(widget_bounds))`
/// 2. Per frame: `renderer.render(dirty.take(), |canvas| { ... })`
pub struct DirtyRect {
    region: Option<Rect>,
}

impl DirtyRect {
    pub const fn new() -> Self { Self { region: None } }

    /// Union `rect` into the accumulated damage region.
    pub fn mark(&mut self, rect: Rect) {
        self.region = Some(match self.region {
            Some(acc) => acc.union(rect),
            None      => rect,
        });
    }

    /// Mark the entire surface as dirty.
    pub fn mark_all(&mut self, w: f32, h: f32) {
        self.region = Some(Rect::new(0.0, 0.0, w, h));
    }

    /// Take the accumulated region and reset to clean.
    ///
    /// Returns `None` if nothing was marked dirty since the last call.
    pub fn take(&mut self) -> Option<Rect> { self.region.take() }

    pub fn is_dirty(&self) -> bool { self.region.is_some() }
}

impl Default for DirtyRect {
    fn default() -> Self { Self::new() }
}
