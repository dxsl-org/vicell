// SPDX-License-Identifier: MIT
//! Space — invisible spacer widget that consumes layout space without painting.

extern crate alloc;

use core::cell::Cell;

use crate::dirty::DirtyRegion;
use crate::event::Event;
use crate::layout::{Constraints, Rect, Size};
use crate::node::ViNode;
use crate::render_ctx::RenderCtx;
use crate::signal::SubscriptionHandle;

/// Expands to exactly `width × height` pixels, painting nothing.
///
/// Useful for adding gaps between widgets in `Row`/`Column` containers.
pub struct Space {
    width:        f32,
    height:       f32,
    bounds_cache: Cell<Rect>,
}

impl Space {
    /// Fixed-size spacer.
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height, bounds_cache: Cell::new(Rect::ZERO) }
    }

    /// Horizontal gap (zero height).
    pub fn w(width: f32) -> Self { Self::new(width, 0.0) }

    /// Vertical gap (zero width).
    pub fn h(height: f32) -> Self { Self::new(0.0, height) }
}

impl ViNode for Space {
    fn layout(&mut self, constraints: Constraints) -> Size {
        let size = constraints.constrain(Size { w: self.width, h: self.height });
        self.bounds_cache.set(Rect::from_origin_size(constraints.origin, size));
        size
    }

    fn bounds(&self) -> Rect { self.bounds_cache.get() }

    // Space is invisible — nothing to paint.
    fn paint(&self, _cx: &mut RenderCtx<'_>) {}

    // Space has no interactive surface.
    fn event(&mut self, _event: &Event) -> bool { false }

    fn collect_dirty_handles(&mut self, _region: DirtyRegion) -> alloc::vec::Vec<SubscriptionHandle> {
        alloc::vec![]
    }
}
