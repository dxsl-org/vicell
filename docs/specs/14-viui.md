# ViCell UI Toolkit: ViUI
**Version**: 1.0 (Definitive — supersedes Slint standard in specs/06-graphics.md §4)
**Status**: Architectural Decision — awaiting G2 implementation
**Last Updated**: 2026-06-07

---

## 1. Quyết định kiến trúc

### Tại sao không dùng Slint / iced / egui

| Library | Lý do loại |
|---------|-----------|
| **Slint** | GPL-3 viral hoặc $1+/device commercial. Không thể xây ViCell ecosystem trên license này — mọi App Cell downstream bị ảnh hưởng. |
| **iced** | `iced_runtime` cần std async executor; `iced_winit` coupling không tách được clean cho bare-metal. |
| **egui (port)** | Pipeline: widget → tessellate triangles → rasterize → pixels. Tessellation là overhead không cần thiết cho software renderer. Per-frame heap alloc. |

### ViUI: Custom toolkit, ViCell-native

ViUI được thiết kế từ đầu cho ViCell's constraints:
- `#![no_std] + alloc` — native, không patch
- Direct pixel rendering (không có triangle/path intermediate)
- Event-driven, không game-loop (0 CPU khi idle)
- Dual-facade API: egui-compatible + iced-compatible
- MIT license — không viral, không per-device fee

---

## 2. Kiến trúc layers

```
┌─────────────────────────────────────────────────────────┐
│                    APP CELL CODE                        │
│                                                         │
│  Immediate Facade (egui-compatible)  │  Elm Facade     │
│  ui.label("Hello")                   │  fn view()      │
│  ui.button("Click").clicked()        │  fn update(msg) │
└──────────────────────┬───────────────┴────────┬────────┘
                       │ reconcile              │ retained
           ┌───────────▼────────────────────────▼──────┐
           │             ViUI Core Engine              │
           │  ┌─────────┐ ┌──────────┐ ┌───────────┐  │
           │  │ Widget  │ │ Layout   │ │  Event    │  │
           │  │  Tree   │ │ Engine   │ │ Dispatch  │  │
           │  │(retained│ │(flexbox- │ │   (Elm    │  │
           │  │  nodes) │ │  lite)   │ │ messages) │  │
           │  └─────────┘ └──────────┘ └───────────┘  │
           └──────────────────┬─────────────────────────┘
                              │
           ┌──────────────────▼──────────────────────┐
           │              ViCanvas                   │
           │  fill_rect / draw_text / clip_push      │
           │  (direct pixel ops — no tessellation)   │
           └──────────────────┬──────────────────────┘
                              │
           ┌──────────────────▼──────────────────────┐
           │      embedded-graphics DrawTarget        │
           │       ← ViSurface::pixels_mut()          │
           └──────────────────┬──────────────────────┘
                              │
                        DamageNotify
                              │
                         Compositor
```

---

## 3. Core Engine

### Widget trait

```rust
// libs/viui/src/widget.rs
pub trait ViWidget: 'static {
    fn layout(&self, cx: &LayoutCx, constraints: Constraints) -> Size;
    fn paint(&self, cx: &mut PaintCx);        // PaintCx wraps ViCanvas
    fn event(&mut self, cx: &mut EventCx, e: &Event) -> EventStatus;
    fn children(&self) -> &[Box<dyn ViWidget>];
}
```

**Retained mode**: widget tree chỉ rebuild khi state thay đổi. Không rebuild toàn bộ mỗi frame như egui.

### Layout Engine

Flexbox-lite: `Column`, `Row`, `Stack`, `Padding`, `SizedBox`. Constraints-based (min/max size), không layout toàn bộ tree khi dirty region nhỏ.

### Event Dispatch (Elm)

```rust
// Elm contract — iced-compatible
pub trait ViApp: 'static {
    type Message: 'static;
    fn view(&self) -> Element<Self::Message>;
    fn update(&mut self, msg: Self::Message);
}
```

`Message` types map tự nhiên sang ViCell IPC messages — không cần adapter layer.

---

## 4. ViCanvas — Direct Pixel Rendering

```rust
pub trait ViCanvas {
    fn fill_rect(&mut self, rect: Rect, color: Color);
    fn draw_text(&mut self, pos: Point, text: &str, style: TextStyle);
    fn draw_image(&mut self, rect: Rect, data: &[u8], fmt: PixelFormat);
    fn draw_line(&mut self, a: Point, b: Point, color: Color, width: u8);
    fn clip_push(&mut self, rect: Rect);
    fn clip_pop(&mut self);
}
```

Implement trên `embedded-graphics DrawTarget`. Không có tessellation pipeline — widget gọi `fill_rect` để paint thẳng vào `&mut [u8]`.

**Tại sao nhanh hơn egui/iced**: egui cần tessellate → rasterize triangles. ViCanvas gọi `memset`/`memcpy` pattern trực tiếp — gần memory bandwidth của hardware.

---

## 5. Dual-Facade API

### Immediate Mode Facade (egui-compatible)

```rust
// libs/viui/src/ui.rs — ~95% API compatibility với egui
impl Ui<'_> {
    pub fn label(&mut self, text: impl Into<String>) -> Response;
    pub fn button(&mut self, text: impl Into<String>) -> Response;
    pub fn text_edit_singleline(&mut self, text: &mut String) -> Response;
    pub fn checkbox(&mut self, checked: &mut bool, label: &str) -> Response;
    pub fn horizontal(&mut self, f: impl FnOnce(&mut Ui));
    pub fn vertical(&mut self, f: impl FnOnce(&mut Ui));
    pub fn add(&mut self, widget: impl ViWidget) -> Response;
}

impl Response {
    pub fn clicked(&self) -> bool;
    pub fn hovered(&self) -> bool;
    pub fn changed(&self) -> bool;
}
```

Developer biết egui có thể dùng ViUI với zero learning curve. Khác biệt duy nhất: không có `eframe::App` — thay bằng `ViApp` trait; không có backend setup — ViCell lo thay.

### Elm Facade (iced-compatible)

```rust
// libs/viui/src/elm.rs — ~90% API compatibility với iced
pub fn column<Msg>(children: Vec<Element<Msg>>) -> Element<Msg>;
pub fn row<Msg>(children: Vec<Element<Msg>>) -> Element<Msg>;
pub fn text<Msg>(content: impl Into<String>) -> Element<Msg>;
pub fn button<Msg>(content: impl Into<String>) -> Element<Msg>;
// macro
viui::column![text("Hello"), button("Click").on_press(Msg::Click)]
```

---

## 6. Text Rendering

### Hai tầng

| Tầng | Crate | Dùng khi | Tốc độ |
|------|-------|----------|--------|
| **Bitmap 8×8** | internal (`libs/ostd/src/font.rs`) | CLI mode, debug text, minimal apps | ~0.001ms/glyph |
| **GlyphAtlas** | `fontdue` + `libs/ostd/src/font_atlas.rs` | UI apps cần scalable font, Unicode | ~0.002ms/glyph (cache hit) |

### GlyphAtlas design

```rust
// libs/ostd/src/font_atlas.rs
pub struct GlyphAtlas {
    data:    Box<[u8]>,              // 512×512 alpha channel
    entries: BTreeMap<GlyphKey, GlyphEntry>,
}

impl GlyphAtlas {
    pub fn prewarm_ascii(&mut self, font: &fontdue::Font, size: f32);
    pub fn get_or_insert(&mut self, font: &fontdue::Font,
                         c: char, size: f32) -> &GlyphEntry;
}
```

Pre-warm ASCII 0x20–0x7E tại app startup (~19ms một lần). Cache hit sau đó = memcpy ~200 bytes — tốc độ bằng bitmap font.

**Tại sao không FreeType**: C FFI, không no_std, vi phạm Law 4 trong App Cells.

---

## 7. Text rendering gap — fontdue vs FreeType vs ab_glyph

fontdue được giữ làm rasterizer vì: no_std native, pure Rust, anti-aliasing tốt. Vấn đề hiện tại là thiếu atlas cache — không phải rasterizer chậm. ab_glyph (egui's rasterizer) là alternative nếu fontdue có vấn đề.

---

## 8. Tích hợp với Compositor

ViUI chạy hoàn toàn trong App Cell. Compositor không biết ViUI tồn tại.

```
App Cell:
  ViApp::view() → ViUI render → ViSurface::pixels_mut()
  ViSurface::damage(dirty_rect) → DamageNotify IPC → Compositor

Compositor:
  Nhận DamageNotify → blend surface vào framebuffer → VirtIO GPU flush
```

Không có round-trip giữa ViUI và Compositor trong render path — chỉ 24-byte DamageNotify sau khi app đã vẽ xong.

---

## 9. Performance Profile

| Scenario | ViUI | egui | iced | Slint |
|----------|------|------|------|-------|
| Idle (no change) | ~0 | ~6-17ms (game loop) | ~0 | ~0 |
| Button press (small dirty) | ~0.05ms | ~6-17ms | ~0.5-2ms | ~0.1-0.3ms |
| Full screen redraw 1080p | ~2-5ms | ~6-17ms | ~8-20ms | ~5-15ms |
| Per-frame alloc | zero | high | low | zero |

ViUI nhanh nhất vì pipeline ngắn nhất (widget → pixels trực tiếp).

---

## 10. Crate location

```
libs/viui/
├── Cargo.toml           # no_std, alloc only
├── src/
│   ├── lib.rs
│   ├── widget.rs        # ViWidget trait
│   ├── layout.rs        # LayoutEngine, Constraints
│   ├── canvas.rs        # ViCanvas trait + FramebufferCanvas
│   ├── event.rs         # Event, EventStatus, Focus
│   ├── response.rs      # Response (clicked/hovered/changed)
│   ├── ui.rs            # Immediate mode facade (egui-compatible)
│   ├── elm.rs           # Elm facade (iced-compatible)
│   ├── theme.rs         # Theme trait, dark/light defaults
│   └── widgets/
│       ├── label.rs
│       ├── button.rs
│       ├── text_edit.rs
│       ├── checkbox.rs
│       ├── scroll_area.rs
│       └── image.rs
```

App Cells dùng: `use viui::prelude::*;` + `use ostd::display::ViSurface;`

---

## 11. Phân tầng theo profile

| Mode | UI usage |
|------|----------|
| **CLI** | Bitmap font only, không load ViUI |
| **Kiosk** | ViUI full-screen single app, GlyphAtlas, no window chrome |
| **Desktop** | ViUI multi-window, window decorations, taskbar — G2 |

---

## 12. Milestones

| Phase | Nội dung | Stage |
|-------|----------|-------|
| P01 | Core Engine (Widget trait, Layout, Event, Elm) | G2 start |
| P02 | ViCanvas + DrawTarget (direct pixel rendering) | G2 start |
| P03 | Immediate Mode Facade (egui Ui API) | G2 start |
| P04 | Basic Widget Set (Label, Button, TextInput, CheckBox, ScrollArea) | G2 start |
| P05 | GlyphAtlas + fontdue scalable text | G2 start |
| P06 | Theming (dark/light, custom palette, `Theme` trait) | G2 |
| P07 | Elm Facade (iced view/update/Message API) | G2 |
| P08 | Animation system (transitions, property animation) | G2 later |

P01–P06 là G2 MVP. P07–P08 là G2 polish.

---

## References

- [specs/06-graphics.md](06-graphics.md) — Compositor + Input architecture
- [libs/ostd/src/display.rs](../../libs/ostd/src/display.rs) — ViSurface API
- [libs/api/src/display.rs](../../libs/api/src/display.rs) — Compositor protocol (DamageNotify, AttachGrant)
- [cells/services/compositor/](../../cells/services/compositor/) — Compositor implementation
