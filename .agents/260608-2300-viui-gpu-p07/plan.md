# ViUI v2 P07 — GPU Command Buffer Renderer

**Plan ID**: 260608-2300-viui-gpu-p07
**Stage**: G2
**Priority**: P2 — enables damage-rect optimization + future hardware GPU execution
**Created**: 2026-06-08
**Status**: ✅ COMPLETE — 2026-06-08
**Depends on**: P02 (ViRenderer trait + FramebufferRenderer), P03-P04 (ViCanvas trait)

---

## Mục tiêu

Thêm `GpuRenderer` — implementation thứ hai của `ViRenderer` — sử dụng **command list pattern**:

1. Paint pass ghi các draw command (không rasterize ngay)
2. Executor nhận command list → thực thi với damage-rect filtering
3. `CpuExecutor`: playback qua `FramebufferCanvas` (output giống hệt hiện tại, nhưng skip vùng unchanged)
4. Kiến trúc mở: `CommandExecutor` trait → future hardware GPU executor không đổi app code

```rust
// App code — giống hệt, không thay đổi:
renderer.render(Some(dirty_rect), &mut |canvas| root.paint(canvas));

// Internally GpuRenderer thay vì rasterize ngay:
//   1. GpuCanvas records → GpuCommandBuffer
//   2. CpuExecutor replays, skipping cmds outside dirty_rect
```

---

## Why command list, not immediate GPU

ViCell G1 (QEMU VirtIO GPU): không có hardware 2D accel primitives — VirtIO GPU chỉ là DMA framebuffer transfer. Command list vẫn dùng CPU rasterization nhưng:
- Skip repaint vùng ngoài `damage: Option<Rect>` → tiết kiệm CPU
- Recorded commands có thể dedup (future): nếu widget chưa dirty, replay cached commands

ViCell G2 (real hardware): `CommandExecutor` trait → add `HwGpuExecutor` mà không đổi app code. Command list là abstraction đúng cho 2D hw accel (Mali DE, PowerVR 2D, RISC-V GPU).

---

## Architecture

```
ViRenderer::render(damage, draw_fn)
       │
       ▼
GpuRenderer<E: CommandExecutor>
       │
       ├─ GpuCanvas (records draw calls)
       │       ↓
       │  GpuCommandBuffer (Vec<GpuCmd>)
       │
       └─ executor.execute(buf, damage)
               │
               ├─ CpuExecutor → FramebufferCanvas → ViSurface → compositor
               └─ (future) HwGpuExecutor → GPU driver cell
```

**Key types:**

| Type | File | Vai trò |
|------|------|---------|
| `GpuCmd` | `gpu_cmd.rs` | Enum — typed draw command |
| `GpuCommandBuffer` | `gpu_cmd.rs` | Vec\<GpuCmd\> recorder |
| `GpuCanvas` | `gpu_canvas.rs` | ViCanvas impl → records to buffer |
| `CommandExecutor` | `executor.rs` | Trait — execute GpuCommandBuffer |
| `CpuExecutor` | `executor.rs` | CommandExecutor → FramebufferCanvas playback |
| `GpuRenderer<E>` | `gpu_renderer.rs` | ViRenderer impl wrapping any CommandExecutor |

---

## Phase Table

| Phase | File | Nội dung | Status |
|-------|------|----------|--------|
| P01 | [phase-01-gpu-cmd-buffer.md](phase-01-gpu-cmd-buffer.md) | `GpuCmd` enum + `GpuCommandBuffer` + `GpuCanvas` | ✅ Done |
| P02 | [phase-02-gpu-renderer.md](phase-02-gpu-renderer.md) | `CommandExecutor` trait + `CpuExecutor` + `GpuRenderer<E>` | ✅ Done |
| P03 | [phase-03-integration.md](phase-03-integration.md) | lib.rs export + viui-demo dual-renderer demo + cargo check | ✅ Done |

P02 depends on P01. P03 depends on P02.

---

## Files Created/Modified

```
libs/viui/src/
├── gpu_cmd.rs        (NEW — GpuCmd enum + GpuCommandBuffer)
├── gpu_canvas.rs     (NEW — GpuCanvas: ViCanvas recorder)
├── executor.rs       (NEW — CommandExecutor trait + CpuExecutor)
├── gpu_renderer.rs   (NEW — GpuRenderer<E: CommandExecutor>)
└── lib.rs            (MODIFY — pub mod gpu_cmd, gpu_canvas, executor, gpu_renderer)

cells/apps/viui-demo/src/main.rs  (MODIFY — GpuRenderer demo alongside FramebufferRenderer)
```

---

## Constraints

- **Law 4**: `libs/viui/` = Cell lib → `#![forbid(unsafe_code)]` — no unsafe in any new file
- **Law 5**: no `mod.rs` — dùng `foo.rs` parallel pattern ✓
- **Law 1**: không chạm `libs/api/` hay `libs/types/` — không cần confirm
- **File size**: mỗi file < 200 lines — `GpuCmd` + `GpuCommandBuffer` cùng file; executor + CPU impl cùng file

---

## Success Criteria

- `cargo check -p viui` passes (tất cả 4 new modules compile)
- `cargo check -p viui-demo` passes (demo dùng cả `FramebufferRenderer` và `GpuRenderer<CpuExecutor>`)
- `cargo clippy -p viui -- -D warnings` clean
- `GpuRenderer<CpuExecutor>` renders identical output to `FramebufferRenderer` (same CPU path)
- `render(Some(rect), ...)` skips commands outside `rect` (damage optimization active)

---

## Risk Assessment

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| `DrawImage` với `pixels: Vec<u8>` — clone cost | Medium | Chấp nhận ở G1; G2 dùng grant pages thay thế |
| `DrawText` với `String` alloc per command | Medium | Acceptable; retained caching ở P08+ |
| `GpuCanvas` clip stack vs `FramebufferCanvas` — behavior mismatch | Low | Unit test fill_rect với clip để verify |
| `ViSurface::pixels_mut()` inside CpuExecutor — borrow scope | Low | Follow FramebufferRenderer pattern; scoped to execute() call |
