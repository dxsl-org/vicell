// SPDX-License-Identifier: MIT
//! GPU draw command types and command buffer recorder.
//!
//! `GpuCmd` encodes one draw operation without executing it. Commands are
//! accumulated into `GpuCommandBuffer` during a paint pass and replayed by
//! a `CommandExecutor` — allowing damage-rect filtering and future hardware
//! GPU execution without changing widget code.

use alloc::{string::String, vec::Vec};
use crate::canvas::{Color, TextStyle};
use crate::layout::{Point, Rect};

// ─── GpuCmd ──────────────────────────────────────────────────────────────────

/// A single recorded draw operation.
#[derive(Debug)]
pub enum GpuCmd {
    /// Fill a pre-clipped solid rectangle.
    FillRect  { rect: Rect, color: Color },
    /// Draw a line from `a` to `b` (Bresenham, handled by executor).
    DrawLine  { a: Point, b: Point, color: Color },
    /// Draw text at `pos`. `style.size_px == 0` = bitmap 8×8 fallback.
    DrawText  { pos: Point, text: String, style: TextStyle },
    /// Blit raw BGRA pixels. Destination may extend beyond current clip —
    /// executor applies clipping during playback.
    DrawImage { dest: Rect, pixels: Vec<u8>, src_stride: u32 },
    /// Zero-alloc path for text ≤ 127 bytes (covers all typical single-line UI strings).
    /// Bytes are always valid UTF-8 — written from `&str` in `GpuCanvas::draw_text`.
    DrawTextShort { pos: Point, bytes: [u8; 128], len: u8, style: TextStyle },
}

impl GpuCmd {
    /// Conservative bounding rect used for damage-rect filtering.
    ///
    /// Returns `None` only if the command is inherently unclippable; callers
    /// that see `None` must always execute the command.
    pub fn bounding_rect(&self) -> Option<Rect> {
        match self {
            GpuCmd::FillRect { rect, .. } => Some(*rect),
            GpuCmd::DrawLine { a, b, .. } => {
                let x  = a.x.min(b.x);
                let y  = a.y.min(b.y);
                let x2 = a.x.max(b.x);
                let y2 = a.y.max(b.y);
                // Min 1px so a horizontal/vertical line still has area.
                Some(Rect { x, y, w: (x2 - x).max(1.0), h: (y2 - y).max(1.0) })
            }
            GpuCmd::DrawText { pos, text, .. } => {
                // 8×8 bitmap font: estimate width = chars × 8px
                Some(Rect { x: pos.x, y: pos.y, w: text.len() as f32 * 8.0, h: 8.0 })
            }
            GpuCmd::DrawImage { dest, .. } => Some(*dest),
            GpuCmd::DrawTextShort { pos, len, .. } => {
                Some(Rect { x: pos.x, y: pos.y, w: *len as f32 * 8.0, h: 8.0 })
            }
        }
    }
}

// ─── GpuCommandBuffer ────────────────────────────────────────────────────────

/// Ordered list of draw commands recorded during one paint pass.
pub struct GpuCommandBuffer {
    cmds: Vec<GpuCmd>,
}

impl GpuCommandBuffer {
    pub fn new() -> Self { Self { cmds: Vec::new() } }

    pub fn push(&mut self, cmd: GpuCmd) { self.cmds.push(cmd); }

    pub fn as_slice(&self) -> &[GpuCmd] { &self.cmds }

    pub fn len(&self) -> usize { self.cmds.len() }

    pub fn is_empty(&self) -> bool { self.cmds.is_empty() }

    /// Clear all commands while retaining the Vec's heap allocation for the next frame.
    pub fn clear(&mut self) { self.cmds.clear(); }
}

impl Default for GpuCommandBuffer {
    fn default() -> Self { Self::new() }
}
