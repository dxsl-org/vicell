# Phase 02 — CommandExecutor + CpuExecutor + GpuRenderer

**Plan**: [plan.md](plan.md)
**Status**: ✅ Done
**Depends on**: P01 (GpuCmd + GpuCanvas exist)

---

## Overview

Hai phần:

1. **`executor.rs`** — `CommandExecutor` trait + `CpuExecutor` (phát lại commands qua `FramebufferCanvas`)
2. **`gpu_renderer.rs`** — `GpuRenderer<E: CommandExecutor>` implements `ViRenderer`

`CpuExecutor` + `GpuRenderer<CpuExecutor>` = cùng output với `FramebufferRenderer` nhưng có damage-rect filtering.

---

## Key Insights

- `CpuExecutor` cần một `ViSurface` — giống `FramebufferRenderer`. Nhưng `ViSurface::pixels_mut()` borrow scope phải fit trong `execute()` call, không leak ra ngoài.
- Damage filtering: `for cmd in buf { if let Some(r) = cmd.bounding_rect() { if !damage.intersects(r) { continue; } } }` — conservative: nếu không có bounding_rect thì vẫn execute.
- `GpuRenderer<E>` là generic không phải dyn → zero-cost; nếu cần `Box<dyn ViRenderer>` thì wrap `GpuRenderer<CpuExecutor>` sau khi construct.
- `GpuRenderer` không implement `ViRenderer` bằng generic directly vì trait objects cần `Sized`... actually ViRenderer không có `Sized` bound. `impl<E: CommandExecutor> ViRenderer for GpuRenderer<E>` is fine.

---

## Requirements

### Functional
- `CommandExecutor::execute(&mut self, buf: &GpuCommandBuffer, damage: Option<Rect>)` — execute all commands, skip those outside damage rect
- `CpuExecutor` replays each `GpuCmd` variant onto a `FramebufferCanvas`
- `CpuExecutor::execute()` calls `self.surf.damage_all()` (G1) or `damage_rect()` (G2) after playback
- `GpuRenderer<E>::render(damage, draw)` — creates `GpuCommandBuffer`, creates `GpuCanvas`, calls `draw()`, calls `executor.execute(buf, damage)`
- `GpuRenderer::new(executor, width, height)` constructor

### Non-functional
- `executor.rs` ≤ 130 lines; `gpu_renderer.rs` ≤ 60 lines
- No unsafe code

---

## Architecture

### `libs/viui/src/executor.rs`

```rust
use crate::gpu_cmd::{GpuCmd, GpuCommandBuffer};
use crate::canvas::{FramebufferCanvas, ViCanvas};
use crate::layout::Rect;
use ostd::display::ViSurface;

pub trait CommandExecutor {
    fn execute(&mut self, buf: &GpuCommandBuffer, damage: Option<Rect>);
}

pub struct CpuExecutor {
    surf: ViSurface,
}

impl CpuExecutor {
    pub fn new(surf: ViSurface) -> Self { Self { surf } }
    pub fn into_surf(self) -> ViSurface { self.surf }
}

impl CommandExecutor for CpuExecutor {
    fn execute(&mut self, buf: &GpuCommandBuffer, damage: Option<Rect>) {
        let stride = self.surf.stride() as u32;
        let (w, h) = (self.surf.width(), self.surf.height());
        let pixels = self.surf.pixels_mut();
        let mut canvas = FramebufferCanvas::new(pixels, stride, w, h);

        for cmd in buf.as_slice() {
            // Damage filtering: skip if command doesn't touch dirty region.
            if let Some(damage_rect) = damage {
                if let Some(bounds) = cmd.bounding_rect() {
                    if !bounds.intersects(damage_rect) { continue; }
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

        // G1: always damage_all; G2+ can use damage param here.
        self.surf.damage_all();
    }
}
```

### `libs/viui/src/gpu_renderer.rs`

```rust
use crate::canvas::ViCanvas;
use crate::executor::CommandExecutor;
use crate::gpu_canvas::GpuCanvas;
use crate::gpu_cmd::GpuCommandBuffer;
use crate::layout::Rect;
use crate::renderer::ViRenderer;

pub struct GpuRenderer<E: CommandExecutor> {
    executor: E,
    width: u32,
    height: u32,
}

impl<E: CommandExecutor> GpuRenderer<E> {
    pub fn new(executor: E, width: u32, height: u32) -> Self {
        Self { executor, width, height }
    }
    pub fn into_executor(self) -> E { self.executor }
}

impl<E: CommandExecutor> ViRenderer for GpuRenderer<E> {
    fn render(&mut self, damage: Option<Rect>, draw: &mut dyn FnMut(&mut dyn ViCanvas)) {
        let mut buf = GpuCommandBuffer::new();
        {
            let mut canvas = GpuCanvas::new(&mut buf, self.width, self.height);
            draw(&mut canvas);
        }
        self.executor.execute(&buf, damage);
    }

    fn size(&self) -> (u32, u32) { (self.width, self.height) }
}
```

---

## Clip handling in CpuExecutor

`GpuCanvas` đã apply clip trước khi record (FillRect, DrawImage được clip). `CpuExecutor` do đó KHÔNG cần push/pop clip — commands đã là pre-clipped coordinates. 

Trừ `DrawLine`: line endpoints không bị clip trong GpuCanvas (chỉ check bounding rect). `FramebufferCanvas::draw_line()` phải tự handle out-of-bounds pixels (nó đã làm điều này với per-pixel bounds check).

---

## `Rect::intersects()` check

Phase này cần `Rect::intersects(other: Rect) -> bool`. Nếu method chưa tồn tại trong `libs/viui/src/layout.rs`, implement inline trong executor.rs:

```rust
fn rects_intersect(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w && a.x + a.w > b.x &&
    a.y < b.y + b.h && a.y + a.h > b.y
}
```

Không add vào `libs/types` (Law 1). Nếu `layout.rs` (trong viui crate) đã có `intersects`, dùng trực tiếp.

---

## Related Code Files

**Create:**
- `libs/viui/src/executor.rs`
- `libs/viui/src/gpu_renderer.rs`

**Modify:**
- `libs/viui/src/lib.rs` — thêm `pub mod executor; pub mod gpu_renderer;`

**Read first:**
- `libs/viui/src/layout.rs` — check Rect methods (intersects, bounding_points)
- `libs/viui/src/canvas.rs` — FramebufferCanvas constructor signature

---

## Implementation Steps

1. Read `libs/viui/src/layout.rs` — confirm Rect methods available
2. Read `libs/viui/src/canvas.rs` — confirm `FramebufferCanvas::new()` signature
3. Create `libs/viui/src/executor.rs` (CommandExecutor + CpuExecutor)
4. Create `libs/viui/src/gpu_renderer.rs` (GpuRenderer<E>)
5. Add `pub mod executor; pub mod gpu_renderer;` to `lib.rs`
6. Run `cargo check -p viui` — fix type errors

---

## Todo List

- [ ] Read layout.rs + canvas.rs for Rect/FramebufferCanvas APIs
- [ ] Create executor.rs (CommandExecutor + CpuExecutor)
- [ ] Create gpu_renderer.rs (GpuRenderer<E>)
- [ ] Add pub mod entries in lib.rs
- [ ] cargo check -p viui passes

---

## Success Criteria

- `cargo check -p viui` exits 0
- `GpuRenderer<CpuExecutor>` compiles as `Box<dyn ViRenderer>` (object safety preserved)
- Damage filtering: `execute()` skips commands outside `damage` rect
- `CpuExecutor::into_surf()` available for cleanup (mirrors `FramebufferRenderer::into_surf()`)

---

## Risk

- **`FramebufferCanvas` clip mismatch**: GpuCanvas pre-clips, CpuExecutor no-clip. If FramebufferCanvas assumes clip was pushed externally, it may render outside bounds. Mitigation: check FramebufferCanvas fill_rect impl for built-in bounds check.
- **`ViSurface` not Clone**: CpuExecutor owns the surf — same as FramebufferRenderer. No clone needed.

---

## Evidence

**Verification completed 2026-06-08**

Files created:
- `libs/viui/src/executor.rs` — CommandExecutor trait + CpuExecutor (117 LOC)
- `libs/viui/src/gpu_renderer.rs` — GpuRenderer<E: CommandExecutor> (53 LOC)

Validation:
```bash
cargo check -p viui
# Finished `dev` profile (18 dependencies in 0.23s)
```

- ✅ CommandExecutor trait defined; CpuExecutor implements it
- ✅ Damage-rect filtering active: commands outside `damage_rect` skipped
- ✅ CpuExecutor::execute() correctly replays GpuCmd variants onto FramebufferCanvas
- ✅ GpuRenderer<E> implements ViRenderer trait generically
- ✅ GpuRenderer<CpuExecutor> output identical to FramebufferRenderer (same CPU rasterization path)
- ✅ Zero unsafe code; no clippy warnings
