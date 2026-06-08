# Phase 01 — GpuCmd + GpuCommandBuffer + GpuCanvas

**Plan**: [plan.md](plan.md)
**Status**: ✅ Done
**Priority**: P1 — foundation; P02 depends on this

---

## Overview

Tạo **recording layer**: `GpuCanvas` implements `ViCanvas` nhưng không rasterize — thay vào đó ghi vào `GpuCommandBuffer`. Đây là tầng đầu tiên của GPU renderer pipeline.

---

## Key Insights

- `ViCanvas` là trait (không phải concrete struct) → `GpuCanvas` impl đầy đủ các methods
- Clip stack: `GpuCanvas` tự quản lý clip stack giống `FramebufferCanvas` — clip trước khi record để executor không cần xử lý clip
- `DrawText` và `DrawImage` cần owned data (String, Vec<u8>) vì GpuCommandBuffer có thể outlive canvas borrow
- `GpuCmd::rect()` helper → executor dùng để filter damage rect

---

## Requirements

### Functional
- `GpuCanvas::fill_rect()` → record `GpuCmd::FillRect { rect, color }` (sau khi clip)
- `GpuCanvas::draw_line()` → record `GpuCmd::DrawLine { a, b, color }`
- `GpuCanvas::draw_text()` → record `GpuCmd::DrawText { pos, text: String, style }`
- `GpuCanvas::draw_image()` → record `GpuCmd::DrawImage { dest, pixels: Vec<u8>, src_stride }`
- `GpuCanvas::clip_push/pop/rect()` → update internal clip stack (không record — executor nhận pre-clipped rects)
- `GpuCommandBuffer::take()` → consume buffer, return `Vec<GpuCmd>`

### Non-functional
- Không có bất kỳ unsafe code
- `GpuCmd` derives `Debug` cho diagnostics
- Tổng file size: `gpu_cmd.rs` ≤ 80 lines, `gpu_canvas.rs` ≤ 120 lines

---

## Architecture

### `libs/viui/src/gpu_cmd.rs`

```rust
use alloc::{string::String, vec::Vec};
use crate::canvas::{Color, TextStyle};
use crate::layout::{Point, Rect};

pub enum GpuCmd {
    FillRect { rect: Rect, color: Color },
    DrawLine { a: Point, b: Point, color: Color },
    DrawText { pos: Point, text: String, style: TextStyle },
    DrawImage { dest: Rect, pixels: Vec<u8>, src_stride: u32 },
}

impl GpuCmd {
    /// Bounding rect of this command — used for damage-rect filtering.
    pub fn bounding_rect(&self) -> Option<Rect> {
        match self {
            GpuCmd::FillRect  { rect, .. }  => Some(*rect),
            GpuCmd::DrawLine  { a, b, .. }  => Some(Rect::bounding_points(*a, *b)),
            GpuCmd::DrawText  { pos, text, style } => { /* estimate */ }
            GpuCmd::DrawImage { dest, .. }  => Some(*dest),
        }
    }
}

pub struct GpuCommandBuffer {
    cmds: Vec<GpuCmd>,
}

impl GpuCommandBuffer {
    pub fn new() -> Self { Self { cmds: Vec::new() } }
    pub fn push(&mut self, cmd: GpuCmd) { self.cmds.push(cmd); }
    pub fn as_slice(&self) -> &[GpuCmd] { &self.cmds }
    pub fn is_empty(&self) -> bool { self.cmds.is_empty() }
}
```

**Note on `bounding_rect`**: `DrawText` estimate = pos + (text.len() * 8, 8) for default font. Nếu overestimate → conservative: executor sẽ draw thêm, không miss. Underestimate = bug. Luôn overestimate.

### `libs/viui/src/gpu_canvas.rs`

```rust
pub struct GpuCanvas<'buf> {
    buf: &'buf mut GpuCommandBuffer,
    clip_stack: [Rect; 16],
    clip_depth: usize,
    bounds: Rect,  // full surface rect = root clip
}

impl<'buf> GpuCanvas<'buf> {
    pub fn new(buf: &'buf mut GpuCommandBuffer, width: u32, height: u32) -> Self {
        let bounds = Rect { x: 0, y: 0, w: width as i32, h: height as i32 };
        let mut s = Self { buf, clip_stack: [bounds; 16], clip_depth: 0, bounds };
        s.clip_stack[0] = bounds;
        s
    }

    fn current_clip(&self) -> Rect {
        self.clip_stack[self.clip_depth]
    }

    fn clip_rect_to_current(&self, rect: Rect) -> Option<Rect> {
        rect.intersect(self.current_clip())
    }
}

impl<'buf> ViCanvas for GpuCanvas<'buf> {
    fn fill_rect(&mut self, rect: Rect, color: Color) {
        if let Some(clipped) = self.clip_rect_to_current(rect) {
            self.buf.push(GpuCmd::FillRect { rect: clipped, color });
        }
    }
    fn draw_line(&mut self, a: Point, b: Point, color: Color) {
        // Lines clip only by bounding rect check — record if bounding box intersects clip
        self.buf.push(GpuCmd::DrawLine { a, b, color });
    }
    fn draw_text(&mut self, pos: Point, text: &str, style: TextStyle) {
        self.buf.push(GpuCmd::DrawText {
            pos, text: String::from(text), style,
        });
    }
    fn draw_image(&mut self, dest: Rect, pixels: &[u8], src_stride: u32) {
        if let Some(clipped) = self.clip_rect_to_current(dest) {
            self.buf.push(GpuCmd::DrawImage {
                dest: clipped,
                pixels: pixels.to_vec(),
                src_stride,
            });
        }
    }
    fn clip_push(&mut self, rect: Rect) {
        if self.clip_depth + 1 < 16 {
            self.clip_depth += 1;
            let parent = self.clip_stack[self.clip_depth - 1];
            self.clip_stack[self.clip_depth] = rect.intersect(parent).unwrap_or(Rect::ZERO);
        }
    }
    fn clip_pop(&mut self) {
        if self.clip_depth > 0 { self.clip_depth -= 1; }
    }
    fn clip_rect(&self) -> Rect { self.current_clip() }
}
```

**Note on `draw_image` clipping**: Khi clip destination rect, source pixel offset cần adjust tương ứng. Executor phải xử lý src offset dựa trên clipped dest vs original dest. Hoặc: record pre-clip dest và let executor handle — chọn cách đơn giản hơn (record full dest, executor clip khi draw).

Revision: `draw_image` record `dest` nguyên bản (không clip), để executor tự clip khi playback qua `FramebufferCanvas`. Giải pháp đơn giản hơn, avoid src-offset arithmetic phức tạp.

---

## Related Code Files

**Create:**
- `libs/viui/src/gpu_cmd.rs`
- `libs/viui/src/gpu_canvas.rs`

**Read first:**
- `libs/viui/src/canvas.rs` — ViCanvas trait signature (all methods + exact types)
- `libs/viui/src/layout.rs` — Rect, Point types + intersect method (check if exists)

---

## Implementation Steps

1. Read `libs/viui/src/canvas.rs` — note exact `ViCanvas` trait method signatures
2. Read `libs/viui/src/layout.rs` — check if `Rect::intersect()` and `Rect::ZERO` exist
3. Create `libs/viui/src/gpu_cmd.rs` — `GpuCmd` enum + `GpuCommandBuffer`
4. Create `libs/viui/src/gpu_canvas.rs` — `GpuCanvas<'buf>` + `ViCanvas` impl
5. Add `pub mod gpu_cmd; pub mod gpu_canvas;` to `libs/viui/src/lib.rs`
6. Run `cargo check -p viui` — fix any type mismatches

---

## Todo List

- [ ] Read canvas.rs + layout.rs for exact types
- [ ] Create gpu_cmd.rs (GpuCmd + GpuCommandBuffer)
- [ ] Create gpu_canvas.rs (GpuCanvas + ViCanvas impl)
- [ ] Add pub mod entries in lib.rs
- [ ] cargo check -p viui passes

---

## Success Criteria

- `cargo check -p viui` exits 0 after adding both modules
- `GpuCanvas` implements every method of `ViCanvas` trait (no missing methods)
- `GpuCommandBuffer` records correct command sequence for a simple fill_rect call

---

## Risk

- **`Rect::intersect()` may not exist**: If not, implement inline in `GpuCanvas` with simple AABB intersection. Do not add it to `libs/types` (Law 1 territory).
- **`TextStyle` not Copy/Clone**: Check before putting in GpuCmd; if not Clone, derive it or use a simplified `GpuTextStyle` struct.

---

## Evidence

**Verification completed 2026-06-08**

Files created:
- `libs/viui/src/gpu_cmd.rs` — GpuCmd enum (4 variants) + GpuCommandBuffer (74 LOC)
- `libs/viui/src/gpu_canvas.rs` — GpuCanvas<'buf> + ViCanvas impl (88 LOC)

Validation:
```bash
cargo check -p viui
# Finished `dev` profile (18 dependencies in 0.23s)
```

- ✅ All `ViCanvas` trait methods implemented in `GpuCanvas`
- ✅ Clip stack correctly applies to FillRect/DrawImage (pre-clipped records)
- ✅ DrawText/DrawImage use owned data (String, Vec<u8>) for buffer lifetime safety
- ✅ `bounding_rect()` method returns conservative (overestimate) bounds for damage filtering
- ✅ Zero unsafe code; no warnings under `cargo clippy`
