# 13 — Peripheral Driver Bus (HAL + Driver Cells)

**Version**: 1.2 (Updated 2026-06-15 — CAN loopback + PWM bit-bang + ADC simulation added)
**Status**: 🚧 PARTIAL — GPIO+UART+I2C+SPI+PWM bit-bang + ADC sim + CAN loopback done (QEMU ARM virt); hardware controllers deferred
**Stage**: G1 (Robot & Embedded) — defining requirement for "complete for robots"
**Design source**: `.agents/reports/brainstorm-260606-0730-peripheral-driver-track.md`
**Roadmap**: [project-roadmap.md](../project-roadmap.md) → "Peripheral Driver Track `[G1]`"

---

## 1. Mục tiêu & Triết lý

Robot/embedded điều khiển **sensor & actuator** qua bus phần cứng (GPIO/UART/I2C/SPI/CAN/PWM/ADC).
Cellos hiện chỉ có VirtIO (block/input/net/gpu) — **thiếu hoàn toàn** lớp bus ngoại vi.

**Quyết định kiến trúc (đúng triết lý Cellos):** driver ngoại vi là **true Driver Cells**
(`#![forbid(unsafe_code)]`), KHÔNG nhồi vào kernel. Lý do:
- **Nano-kernel**: kernel giữ <10K LOC; driver không phình TCB.
- **LBI**: bug driver bị cô lập trong Cell, không deref được con trỏ ngoài vùng được cấp.
- **Never-die**: driver Cell sập → supervisor restart, kernel sống (xem [12-reliability.md](12-reliability.md)).
- **Độc lập nâng cấp**: hot-swap từng driver.

Cell cấm `unsafe` → không deref MMIO thô. Giải: **safe-MMIO accessor** (unsafe ẩn trong `ostd`,
thư viện tin cậy) + **Resource Registry** cấp vùng MMIO độc quyền cho Cell.

---

## 2. Ba lớp thành phần

```
┌─────────────────────────────────────────────────────────┐
│ Driver Cell (cells/drivers/gpio, uart, …)                 │  #![forbid(unsafe_code)]
│  - impl ViGpio / ViUart …                                 │
│  - sở hữu MmioRegion (cấp qua grant)                      │
│  - exclusive RT: poll/đọc trực tiếp; shared: phục vụ IPC  │
└───────────────┬───────────────────────────┬───────────────┘
                │ safe-MMIO (ostd)          │ typed IPC (postcard)
┌───────────────▼───────────┐   ┌───────────▼───────────────┐
│ ostd::mmio::MmioRegion     │   │ libs/api::periph (enums)   │
│  - read/write_volatile<T>  │   │  PeriphRequest/Response    │
│  - bounds-checked, unsafe  │   └────────────────────────────┘
│    ẩn trong ostd           │
└───────────────┬────────────┘
┌───────────────▼─────────────────────────────────────────┐
│ Kernel: Resource Registry (kernel/src/resource_registry) │
│  - request_mmio(base,size,cap) → MmioRegion | Rejected   │
│  - BTreeMap<MmioRange, CellId> độc quyền                 │
│  - release-on-exit (hook vào Cell-exit cleanup)          │
└──────────────────────────────────────────────────────────┘
```

---

## 3. HAL Traits (libs/api hoặc hal/traits) — bám khuôn `SerialPort`

```rust
// ViGpio — digital I/O
pub trait ViGpio {
    fn set_direction(&mut self, pin: u8, dir: PinDir) -> ViResult<()>;
    fn write_pin(&mut self, pin: u8, level: bool) -> ViResult<()>;
    fn read_pin(&self, pin: u8) -> ViResult<bool>;
    fn enable_edge_irq(&mut self, pin: u8, edge: Edge) -> ViResult<()>; // hard-RT event
}

// ViUart — mở rộng SerialPort hiện có (init/send/receive) thêm cấu hình
pub trait ViUart: SerialPort {
    fn configure(&mut self, baud: u32, cfg: UartConfig) -> ViResult<()>;
}
```
- ✅ **`ViI2c`** (`hal/traits/i2c/src/lib.rs`) — synchronous master, `write`/`read`/`write_read`. Implemented 2026-06.
- ✅ **`ViSpi`** (`hal/traits/spi/src/lib.rs`) — synchronous master Mode 0, `cs_select`/`cs_deselect`/`transfer`/`write`. Implemented 2026-06-13.
- ✅ **`ViPwm`** (`hal/traits/pwm/src/lib.rs`) — `set_frequency`/`set_duty`/`enable`/`disable`/`tick`. Bit-bang over GPIO. Implemented 2026-06-15.
- ✅ **`ViAdc`** (`hal/traits/adc/src/lib.rs`) — `read_raw`/`max_value`/`num_channels`/`to_millivolts`. Simulation driver. Implemented 2026-06-15.
- ✅ **`ViCan`** (`hal/traits/can/src/lib.rs`) — `configure`/`send_frame`/`recv_frame` + `CanFrame` type. Loopback driver. Implemented 2026-06-15.
- 📋 Defer: MMIO hardware controllers for CAN/ADC (require manifest v2 for new flags — see §8).
- ⚠️ Traits live in `hal/traits/` (NOT `libs/api`) → no Law 1 change. Driver Cells dep on the trait crate directly.

---

## 4. Safe-MMIO Accessor (ostd)

```rust
// ostd::mmio — unsafe ẩn trong lib tin cậy (hợp Law 4)
pub struct MmioRegion { base: usize, len: usize } // constructor crate-private

impl MmioRegion {
    /// Bounds-checked. # Errors: ViError::OutOfBounds nếu offset+size > len.
    pub fn read<T: Copy>(&self, offset: usize) -> ViResult<T>;
    pub fn write<T: Copy>(&self, offset: usize, val: T) -> ViResult<()>;
    // SAFETY (nội bộ ostd): base+offset đã bounds-check; volatile chống reorder.
}
```
- Cell **không tạo được** `MmioRegion` trực tiếp → chỉ nhận qua grant từ kernel.
- Mọi truy cập offset-checked → không OOB. `read_volatile/write_volatile` chống compiler reorder.

---

## 5. Resource Registry (kernel)

- `request_mmio(base, size) -> ViResult<MmioRegion>`: cấp **độc quyền**. Nếu range đã bị chiếm → `Rejected`.
- Lưu `BTreeMap<MmioRange, CellId>`. Chống 2 driver ghi cùng thanh ghi (thảm họa SAS — spec [04-hardware.md](04-hardware.md) §4).
- **Release-on-exit**: khi Cell exit/bị kill → giải phóng range. Thiết kế dạng **hook** gọi từ Cell-exit cleanup, KHÔNG sửa trực tiếp exit path (tránh chồng never-die ForceExit/`terminate` đang sửa).
- **v1**: range **hardcode** cho QEMU ARM virt (PL061 GPIO @0x0903_0000, PL011 UART @0x0900_0000). **DTB discovery** để v2.

---

## 6. Mô hình ngắt / RT loop

Ba lớp tín hiệu, ba cơ chế (chung một nền: RT Cell `spawn_pinned` + sở hữu MMIO độc quyền):

| Lớp tín hiệu | Ví dụ | Cơ chế | v1? |
|---|---|---|---|
| **Soft-RT** | UART RX, sensor định kỳ, telemetry | **Async Waker Dispatch** (spec 04 §3): kernel top-half ack+`wake()`, Cell bottom-half async | ✅ implement |
| **Hard-RT chu kỳ** | PID loop, software PWM, bit-bang, quadrature | **Pinned-poll**: `spawn_pinned` Cell đọc data register qua safe-MMIO, **không IRQ**, latency thấp nhất | ✅ implement |
| **Hard-RT sự kiện** | limit switch, e-stop, encoder index, fault pin | **SSIP-preempt**: kernel ack edge IRQ → `pend_preempt_if_needed()` (Phase 25-3) preempt tức thì tới RealTime Cell | 📋 **design only — defer** |

> **Lý do defer SSIP-preempt**: nối dây IRQ→Cell nằm ngay trap/IRQ dispatch — vùng never-die (Phase 26)
> đang sửa. Implement sau khi kernel ổn định. Pinned-poll phủ được hard-RT chu kỳ mà gần như không đụng kernel.

**Pinned-poll không cần IPC trong vòng nóng**: RT Cell sở hữu MMIO độc quyền → poke thẳng register
qua safe-MMIO. IPC chỉ dùng lúc grant ban đầu. Đây là cách Cellos đạt **vừa cách ly vừa latency thấp**
mà KHÔNG cần direct-IPC vtable (Phase 27-3, vốn là G2).

---

## 7. IPC Contract (ngoại vi dùng chung)

> **Implementation note (v1):** I2C and SPI v1 use the **rlib-consumed-by-app** pattern:
> `BitBangI2c<G>` and `BitBangSpi<G>` are rlib crates (`driver-i2c-gpio`, `driver-spi-gpio`)
> generic over `ViGpio`. The demo/test app owns the GPIO MMIO directly and calls the trait
> in-process — **no IPC broker, no `libs/api` change**. This is the correct pattern for
> single-app bus ownership (KISS/YAGNI). The IPC broker below applies only to
> **multi-Cell shared-bus** scenarios (future: I2C sensor hub serving multiple Cells).

Chỉ áp dụng khi nhiều Cell chia sẻ 1 ngoại vi qua **driver Cell môi giới** (vd bus I2C chung nhiều sensor):

```rust
// libs/api::periph — typed postcard, y khuôn VfsRequest (Milestone 3.3) + net
pub enum PeriphRequest  { GpioWrite{pin,level}, GpioRead{pin}, UartWrite(Box<[u8]>), … }
pub enum PeriphResponse { Ok, Level(bool), Data(Box<[u8]>), Err(ViError) }
```
- Tái dùng pattern typed-IPC đã chứng minh (DRY). Versionable, type-safe.
- Owned buffer (`Box<[u8]>`) cho async (Law 2).
- KHÔNG cần đường raw syscall ở v1.

---

## 8. Capability Model

- **Thiết kế đầy đủ: ZST cap tokens** (`GpioCap(())`, `UartCap(())` — constructor kernel-only,
  như `BlockIoCap` của Phase 26). Gate `request_mmio` + `map_mmio` theo cap.
- **v1 gate tạm qua ELF manifest (Phase 30 ✅ đã xong)**: Cell khai báo cap `gpio`/`uart` trong
  `__Cellos_manifest`; `spawn_from_path` enforce. **KHÔNG phụ thuộc Phase 26** (đang chạy session khác).
- Chuyển sang ZST cap khi Phase 26 land. Granularity: per-bus (v1), cân nhắc per-pin (v2).

---

## 9. Scope v1 (verify-functionally)

**v1 = GPIO + UART + I2C + SPI bit-bang trên QEMU ARM virt** (PL061 GPIO + PL011 UART → verify không cần board thật):
1. ✅ `ostd::mmio::MmioRegion` + bounds-check.
2. ✅ Resource Registry tối thiểu (hardcode ARM virt ranges) + release-on-exit hook.
3. ✅ Trait `ViGpio` + `ViUart`; driver Cell `cells/drivers/gpio` (PL061), `cells/drivers/serial` (PL011).
4. ✅ Async Waker cho UART RX; pinned-poll demo cho GPIO.
5. ✅ ELF-manifest cap gating (gpio/uart).
6. ✅ Integration test trên ARM virt: set pin → read pin; UART loopback.
7. ✅ **I2C bit-bang** (`hal/traits/i2c`, `cells/drivers/i2c-gpio`, `cells/apps/sensor-demo`) — gated by `gpio` manifest cap; pins 0=SCL/1=SDA.
8. ✅ **SPI bit-bang** (`hal/traits/spi`, `cells/drivers/spi-gpio`, `cells/apps/spi-demo`) — gated by `gpio` manifest cap; pins 2=MOSI/3=MISO/4=SCK/5=CS; Mode 0 MSB-first.
   - QEMU note: MISO floats → `transfer()` reads 0x00 (expected); `write()` fully validates TX path.
9. ✅ Integration test `tests/integration/tests/periph-i2c-spi.rs` — asserts SPI TX probe + I2C banner.
10. ✅ **PWM bit-bang** (`hal/traits/pwm`, `cells/drivers/pwm-gpio`, `cells/apps/pwm-demo`) — gated by `gpio` manifest cap; channel N = pin N; counter-based tick(); sweeps duty 0→100% on pin 6.
11. ✅ **ADC simulation** (`hal/traits/adc`, `cells/drivers/adc-sim`, `cells/apps/adc-demo`) — no MMIO, no cap; triangle-wave ramp with optional deterministic noise; 3 channels, 5 iterations.
12. ✅ **CAN loopback** (`hal/traits/can`, `cells/drivers/can-loopback`, `cells/apps/can-demo`) — no MMIO, no cap; fixed 32-frame ring buffer; `CanFrame` (11/29-bit); validates round-trip TX→RX of 5 frames at 500 kbps.
13. ✅ Integration test `tests/integration/tests/periph-can-pwm-adc.rs` — asserts PWM duty + ADC ch0 + CAN RX probes.

**Defer (v2+, real board)**: hardware SPI controller (PL022), hardware I2C controller (DW I2C); MMIO CAN controller (SJA1000/STM32 bxCAN); hardware ADC (SAR); SSIP-preempt; ZST cap; DTB discovery; per-pin granularity; multi-Cell shared-bus IPC broker.
**Manifest v2 needed for MMIO CAN/ADC**: `flags: u8 → u16` (Law 1 ABI bump — separate plan, requires 2× confirmation).

---

## 10. G1 Graduation Contribution

- GPIO + UART chạy trên QEMU ARM virt ✅
- **Reference robot demo**: vòng sensor→compute→actuator (GPIO) + telemetry MQTT chạy end-to-end (tiêu chí tốt-nghiệp G1).

---

## 11. Chồng lấn với session never-die (Phase 26) — đã decouple

| Vùng | Trạng thái |
|---|---|
| `ostd::mmio` safe-MMIO | ✅ Không đụng kernel — an toàn song song |
| Resource Registry (module mới) | ✅ Module riêng `kernel/src/resource_registry.rs` — ít đụng |
| Release-on-exit hook | ⚠️ Đụng Cell-exit cleanup (never-die ForceExit) → **thiết kế hook, implement sau** |
| SSIP-preempt (IRQ path) | ⚠️ Đụng trap/IRQ dispatch → **defer tới khi never-die ổn** |
| ZST cap gating | ⚠️ Phụ thuộc Phase 26 → **v1 dùng ELF manifest, chuyển sau** |

→ **Phần v1 implement (safe-MMIO + Registry module + GPIO/UART Cell + manifest gating) gần như không chồng.**

## See Also
- [04-hardware.md](04-hardware.md) — HAL traits, Async Waker Dispatch, Resource Registry
- [05-application.md](05-application.md) — Cell tiers & isolation
- [12-reliability.md](12-reliability.md) — never-die / supervisor restart
- Phase 30 (ELF capability manifests) — cap gating mechanism reused here
- Phase 25 (priority scheduler, spawn_pinned, SSIP) — RT foundation reused here
