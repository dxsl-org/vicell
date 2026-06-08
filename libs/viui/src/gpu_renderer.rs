// SPDX-License-Identifier: MIT
//! `GpuRenderer<E>` — `ViRenderer` implementation backed by a `CommandExecutor`.
//!
//! The paint closure runs against a `GpuCanvas` (recording mode), then the
//! accumulated `GpuCommandBuffer` is handed to the executor for playback with
//! damage-rect filtering. Widget code is unchanged — it calls the same
//! `ViCanvas` trait methods regardless of the active backend.

use crate::canvas::ViCanvas;
use crate::executor::CommandExecutor;
use crate::gpu_canvas::GpuCanvas;
use crate::gpu_cmd::GpuCommandBuffer;
use crate::layout::Rect;
use crate::renderer::ViRenderer;

// ─── GpuRenderer ─────────────────────────────────────────────────────────────

/// Generic renderer that records draw commands and replays via `E`.
///
/// Use `GpuRenderer<CpuExecutor>` as a drop-in replacement for
/// `FramebufferRenderer` with damage-rect optimization enabled.
/// Swap `E` for a hardware GPU executor in G2+ without touching widget code.
pub struct GpuRenderer<E: CommandExecutor> {
    executor: E,
    width:    u32,
    height:   u32,
    buf:      GpuCommandBuffer,
}

impl<E: CommandExecutor> GpuRenderer<E> {
    pub fn new(executor: E, width: u32, height: u32) -> Self {
        Self { executor, width, height, buf: GpuCommandBuffer::new() }
    }

    /// Unwrap the executor (e.g. to reclaim the inner `ViSurface`).
    pub fn into_executor(self) -> E { self.executor }
}

impl<E: CommandExecutor> ViRenderer for GpuRenderer<E> {
    fn render(&mut self, damage: Option<Rect>, draw: &mut dyn FnMut(&mut dyn ViCanvas)) {
        self.buf.clear();
        {
            let mut canvas = GpuCanvas::new(&mut self.buf, self.width, self.height);
            draw(&mut canvas);
        }
        // NLL field split: &self.buf (immutable) + &mut self.executor separate fields
        self.executor.execute(&self.buf, damage);
    }

    fn size(&self) -> (u32, u32) { (self.width, self.height) }
}
