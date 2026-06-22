# Cellos Display / Compositor API

The Compositor Cell (`cells/services/compositor/`) manages a z-ordered set of
cell surfaces, blends them in software, and flushes dirty regions to the VirtIO
GPU via the `GpuFlush` kernel syscall.

---

## Architecture

```
Consumer cell (shell / app)
        │  IPC: CreateSurface, WritePixels, DamageSurface, …
        ▼
Compositor Cell (cells/services/compositor/)
        ├─ surface_table: CapId → SurfaceState { pixels, pos, damage }
        ├─ z_order: paint order list (bottom → top)
        ├─ render loop (≈ 30 FPS, damage-driven):
        │    blit dirty surfaces into screen FB
        │    call sys_gpu_flush(dirty_rect)
        │                    │
        ▼                    ▼
Kernel GpuFlush syscall (id 300)
        │  copies pixels + calls gpu.flush()
        ▼
VirtIO GPU kernel driver (virtio_gpu.rs)
        │
        ▼
QEMU display (SDL window / VNC)
```

---

## Pixel Format

All surfaces and the screen framebuffer use **BGRA8888** (4 bytes per pixel):
- byte 0: Blue
- byte 1: Green
- byte 2: Red
- byte 3: Alpha (currently unused by compositor — treated as opaque)

Row major, left-to-right, top-to-bottom.

---

## Compositor IPC Protocol

All messages are sent to the compositor's IPC endpoint
(`api::display::COMPOSITOR_ENDPOINT = 5`).

### Request envelope

```
byte[0]   = opcode
byte[1..] = opcode-specific payload
```

### Opcodes

| Opcode | Name | Payload | Reply |
|--------|------|---------|-------|
| `0x01` | `CREATE_SURFACE` | `w: u32 LE, h: u32 LE` | `CapId (u64 LE)`, 0 = error |
| `0x02` | `WRITE_PIXELS` | `cap: u64, x: i32, y: i32, w: u32, h: u32, pixel_data` | — |
| `0x03` | `DAMAGE_SURFACE` | `cap: u64, x: i32, y: i32, w: u32, h: u32` | — |
| `0x04` | `MOVE_SURFACE` | `cap: u64, x: i32, y: i32` | — |
| `0x05` | `RAISE_SURFACE` | `cap: u64` | — |
| `0x06` | `DESTROY_SURFACE` | `cap: u64` | `0x00` ok |
| `0x10` | `GET_SCREEN_SIZE` | — | `w: u32, h: u32` |
| `0xFE` | `DUMP_FB` | — | raw BGRA pixels (debug only) |

### Notes

- **WRITE_PIXELS** does not trigger a render; call **DAMAGE_SURFACE** after writing
  to schedule the region for the next frame.
- **DAMAGE_SURFACE** coordinates are surface-local (0,0 = top-left of the surface).
- Surfaces are automatically given an initial position of (0, 0).  Use **MOVE_SURFACE**
  to reposition before the first damage.
- Destroying a surface does not flush; the compositor composites the hole on the
  next render tick.

---

## Surface Capability Lifecycle

```
CREATE_SURFACE(w, h) → CapId N
       │
       ├─ WRITE_PIXELS(N, x, y, pw, ph, data)
       ├─ DAMAGE_SURFACE(N, x, y, pw, ph)   ← triggers redraw
       ├─ MOVE_SURFACE(N, new_x, new_y)
       ├─ RAISE_SURFACE(N)
       │
       └─ DESTROY_SURFACE(N)                 ← CapId freed
```

---

## GpuFlush Kernel Syscall

Internal to the compositor; cells should not call this directly.

| Field | Description |
|-------|-------------|
| Syscall ID | 300 (`ViSyscall::GpuFlush`) |
| a0 | data_ptr — pointer to BGRA8888 pixel buffer |
| a1 | data_len — byte length (must equal w×h×4) |
| a2 | xy packed — `(x & 0xFFFF) << 16 | (y & 0xFFFF)` |
| a3 | wh packed — `(w & 0xFFFF) << 16 | (h & 0xFFFF)` |

The kernel validates `data_len >= w*h*4`, copies pixels row-by-row into the VirtIO
GPU backing buffer, then calls `gpu.flush()`.

---

## Render Loop

The compositor renders at up to **30 FPS** (≈ 33 ms interval at 10 MHz mtime).
On each tick:

1. Collect the union of all surface `damage` rects.
2. Re-blit all surfaces overlapping the dirty union (bottom → top).
3. Call `sys_gpu_flush(dirty_rect)`.
4. Clear all surface damage flags.

A tick is skipped if no surface has a non-empty damage rect.

---

## Screen Resolution

Default: **1024 × 768** (`FALLBACK_WIDTH` × `FALLBACK_HEIGHT` in `api::display`).

The kernel probes the actual resolution from VirtIO GPU `GET_DISPLAY_INFO` during
`init_driver()`.  The compositor cell queries `GET_SCREEN_SIZE` on startup to size
its framebuffer.

---

## GPU Driver Cell

`cells/drivers/gpu/src/lib.rs` provides two helper functions for cells that need
direct GPU access (rare; normally cells go through the compositor):

```rust
// Flush a BGRA8888 region from a cell-owned buffer.
pub fn flush_rect(pixels: &[u8], rect: Rect) -> ViResult<()>

// Fill a solid colour without managing a pixel buffer.
pub fn fill_rect(rect: Rect, rgba: u32) -> ViResult<()>
```

---

## Files

| File | Purpose |
|------|---------|
| `libs/api/src/display.rs` | `Rect`, `PixelFormat`, `SurfaceCap`, opcode constants |
| `cells/services/compositor/src/lib.rs` | IPC receive loop + message dispatch |
| `cells/services/compositor/src/surface_table.rs` | CapId → surface state map |
| `cells/services/compositor/src/z_order.rs` | Ordered paint list |
| `cells/services/compositor/src/render.rs` | Blending + damage-driven GPU flush |
| `cells/drivers/gpu/src/lib.rs` | `flush_rect` / `fill_rect` helpers |
| `kernel/src/task/drivers/virtio_gpu.rs` | VirtIO GPU init + `GpuFlush` handler |
| `tests/integration/compositor_basic.rs` | QEMU-driven smoke tests |
