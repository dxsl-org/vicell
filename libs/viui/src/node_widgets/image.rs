// SPDX-License-Identifier: MIT
//! Image — renders a raw BGRA8888 pixel buffer into a fixed-size slot.

extern crate alloc;
use alloc::{sync::Arc, vec::Vec};
use core::cell::Cell;

use crate::dirty::DirtyRegion;
use crate::event::Event;
use crate::layout::{Constraints, Rect, Size};
use crate::node::ViNode;
use crate::render_ctx::RenderCtx;
use crate::signal::{Signal, SubscriptionHandle};

/// Renders a static or dynamic BGRA8888 image.
///
/// `data` is a `Signal<Option<Arc<[u8]>>>` — `None` renders nothing (widget
/// occupies space but remains transparent). `width` × `height` are fixed at
/// construction time and determine layout size; `src_stride = width * 4` is
/// assumed (tightly-packed BGRA rows).
///
/// # Invariant
///
/// `data` must contain at least `height * width * 4` bytes when `Some`.
/// Out-of-bounds reads are silently skipped by `FramebufferCanvas::draw_image`.
pub struct Image {
    pub data:     Signal<Option<Arc<[u8]>>>,
    width:        u32,
    height:       u32,
    bounds_cache: Cell<Rect>,
}

impl Image {
    /// Create an image widget with a mutable data signal.
    pub fn new(data: Signal<Option<Arc<[u8]>>>, width: u32, height: u32) -> Self {
        Self {
            data,
            width,
            height,
            bounds_cache: Cell::new(Rect::ZERO),
        }
    }

    /// Convenience: wrap a static BGRA slice (no dynamic updates needed).
    pub fn static_bgra(data: Arc<[u8]>, width: u32, height: u32) -> Self {
        Self::new(Signal::new(Some(data)), width, height)
    }
}

impl ViNode for Image {
    fn layout(&mut self, constraints: Constraints) -> Size {
        let size = constraints.constrain(Size {
            w: self.width  as f32,
            h: self.height as f32,
        });
        self.bounds_cache.set(Rect::from_origin_size(constraints.origin, size));
        size
    }

    fn bounds(&self) -> Rect { self.bounds_cache.get() }

    fn paint(&self, cx: &mut RenderCtx<'_>) {
        // canvas.draw_image(dest: Rect, pixels: &[u8], src_stride: u32)
        // src_stride = width * 4 for tightly-packed BGRA rows.
        if let Some(pixels) = &*self.data.get() {
            let dest = self.bounds_cache.get();
            cx.canvas.draw_image(dest, pixels, self.width * 4);
        }
    }

    // Images are non-interactive by default.
    fn event(&mut self, _event: &Event) -> bool { false }

    fn collect_dirty_handles(&mut self, region: DirtyRegion) -> Vec<SubscriptionHandle> {
        let bounds = self.bounds_cache.get();
        let h = self.data.subscribe(move || { region.borrow_mut().mark(bounds); });
        alloc::vec![h]
    }
}
