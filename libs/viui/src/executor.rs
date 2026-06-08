// SPDX-License-Identifier: MIT
//! `CommandExecutor` trait + `CpuExecutor` — CPU playback backend.
//!
//! `CommandExecutor` decouples the command recorder (`GpuCanvas`) from the
//! actual drawing strategy, letting `GpuRenderer` drive either a CPU
//! rasterizer today or a hardware GPU backend in G2+.
//!
//! # Damage-rect filtering
//! When `damage` is `Some(rect)`, commands whose bounding rect does not
//! intersect `rect` are skipped. `None` = full repaint (skip nothing).
//! Over-estimation of bounding rects is safe; under-estimation causes
//! missing pixels. `GpuCmd::bounding_rect()` always over-estimates.

use crate::canvas::{FramebufferCanvas, ViCanvas};
use crate::gpu_cmd::{GpuCmd, GpuCommandBuffer};
use crate::layout::Rect;
use ostd::display::ViSurface;

// ─── CommandExecutor ─────────────────────────────────────────────────────────

/// Executes a recorded `GpuCommandBuffer`, optionally constrained to a damage rect.
pub trait CommandExecutor {
    /// Replay all commands in `buf`, skipping those outside `damage` if set.
    fn execute(&mut self, buf: &GpuCommandBuffer, damage: Option<Rect>);
}

// ─── CpuExecutor ─────────────────────────────────────────────────────────────

/// G1 CPU executor: replays `GpuCmd`s via `FramebufferCanvas` + `ViSurface`.
///
/// Produces identical output to `FramebufferRenderer` but skips commands
/// outside the supplied damage rect, reducing CPU rasterization work.
pub struct CpuExecutor {
    surf: ViSurface,
}

impl CpuExecutor {
    pub fn new(surf: ViSurface) -> Self { Self { surf } }

    /// Unwrap the inner `ViSurface` (e.g. for IPC cleanup after app exit).
    pub fn into_surf(self) -> ViSurface { self.surf }
}

impl CommandExecutor for CpuExecutor {
    fn execute(&mut self, buf: &GpuCommandBuffer, damage: Option<Rect>) {
        let stride = self.surf.stride() as u32;
        let (w, h) = (self.surf.width(), self.surf.height());
        let pixels = self.surf.pixels_mut();
        let mut canvas = FramebufferCanvas::new(pixels, stride, w, h);

        for cmd in buf.as_slice() {
            // Skip command if its bounding rect doesn't touch the damage area.
            if let Some(damage_rect) = damage {
                if let Some(bounds) = cmd.bounding_rect() {
                    if !bounds.intersects(damage_rect) {
                        continue;
                    }
                }
            }
            match cmd {
                GpuCmd::FillRect { rect, color } =>
                    canvas.fill_rect(*rect, *color),
                GpuCmd::DrawLine { a, b, color } =>
                    canvas.draw_line(*a, *b, *color),
                GpuCmd::DrawText { pos, text, style } =>
                    canvas.draw_text(*pos, text, *style),
                GpuCmd::DrawImage { dest, pixels, src_stride } =>
                    canvas.draw_image(*dest, pixels, *src_stride),
            }
        }

        // G1: always damage_all; G2+ can flip only the damage rect.
        self.surf.damage_all();
    }
}
