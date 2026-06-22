# Cellos Architecture: Hardware Layer
**Version**: 0.3 (Universal HAL & Multi-Arch Strategy)
**Status**: Definitive

---

## 1. Multi-Architecture Strategy (The "Trait" Contract)
Cellos không phụ thuộc vào một kiến trúc CPU cụ thể. Mọi tương tác phần cứng được trừu tượng hóa qua crate `hal-core`.

### Trait-Based Abstraction
| Trait | Vai trò | Đặc điểm đa kiến trúc |
| :--- | :--- | :--- |
| **`Arch`** | Quản lý Context, Paging | Tự thích nghi bit-width (usize) và chuẩn phân trang (Sv32/39/48). |
| **`Interrupt`** | Đăng ký và điều phối ngắt | Hỗ trợ PLIC (RISC-V), GIC (ARM) hoặc APIC (x86). |
| **`Timer`** | Quản lý thời gian thực | Cung cấp độ phân giải cao cho Scheduler. |

## 2. Platform HAL vs. Device Driver Cells
* **Platform HAL**: Được biên dịch **cùng** Nano Kernel. Chịu trách nhiệm khởi tạo CPU, RAM và các thành phần cốt lõi.
* **Driver Cells**: Được nạp động dưới dạng **Cells**. Chịu trách nhiệm cho các ngoại vi (NIC, GPU, cảm biến Robot).

### ~~Chiến lược WASM Sandboxed Drivers~~ _(DROPPED — 2026-06-06)_
WASM Tier-2 đã bị loại khỏi official stack. C driver isolation dùng **Tier 1b FFI** (Newlib shim) thay thế — xem [specs/05-application.md §3](05-application.md). `WasmDriverRuntime` Cell không implement.

## 3. Interrupt Model: "Async Waker Dispatch"
Cellos sử dụng mô hình ngắt bất đồng bộ để tối ưu độ trễ.
1. **Top-Half (Kernel)**: Nhận ngắt cứng, Ack IRQ nhanh nhất có thể và gọi `waker.wake()` tương ứng.
2. **Bottom-Half (Cell)**: Driver Cell xử lý ngắt trong một `async task`. Việc chuyển ngữ cảnh (Context Switch) được tối ưu hóa bằng cách chạy trực tiếp trong SAS.

## 4. Resource Registry (MMIO Isolation)
Trong SAS, việc hai driver cùng ghi vào một địa chỉ phần cứng là thảm họa.
* **Registry**: Kernel quản lý danh sách MMIO dựa trên **Device Tree (DTB)**.
* **Exclusive Access**: Driver phải gọi `kernel.request_mmio(base, size)`. Nếu vùng nhớ đã bị chiếm, Kernel sẽ từ chối cấp phát.

## 5. SMP & Real-Time Affinity
* **Work Stealing**: Scheduler tự động cân bằng tải giữa các core.
* **Affinity**: Các tác vụ điều khiển robot cực kỳ nhạy cảm có thể dùng `spawn_pinned(core_id)` để chiếm quyền ưu tiên tuyệt đối trên một core cụ thể, tránh bị các tác vụ AI làm gián đoạn.

## 6. Deadlock Watchdog
Vì dùng chung bộ nhớ, việc tranh chấp Lock giữa các Cell là rủi ro hiện hữu.
* **Cơ chế**: Một tác vụ nền (Low-priority task) định kỳ quét **Resource Graph**.
* **Xử lý**: Nếu phát hiện vòng lặp (Cycle), hệ thống sẽ chủ động `panic` và reload Cell có độ ưu tiên thấp nhất để giải phóng tài nguyên.

---

## 7. Hardware Support Matrix — Chipsets & Drivers by Stage

> Decided 2026-06-06. Source: `.agents/reports/brainstorm-260606-2205-chipset-driver-strategy.md`

### Target platforms

| Stage | CPU arch | Dev/test | Real board |
|-------|----------|----------|-----------|
| G1 | ARM64 + RV64 | QEMU ARM virt (**QEMU-first**) | RPi 4 (BCM2711) → VisionFive2 (JH7110) |
| G1 sub-track | RV32 | QEMU RV32 | SiFive E21 / CHERIoT-Nano |
| G2 | RV64 | Milk-V Pioneer (X60, now) | Alibaba C930 (2026) |
| G2 | x86_64 | QEMU x86_64 | x86 PC (when G2 starts) |
| G3 | ARM64 | Radxa ROCK 5 / OrangePi 5+ (RK3588) | — |
| G3 | RV64 | — | SiFive P870 + X390 (Q2 2026) |

**QEMU-first rule:** HAL traits (`ViGpio`, `ViUart`, …) must be **board-agnostic** from v1. Adding a new real-board implementation must require zero kernel changes — only a new `impl ViGpio for Bcm2711Gpio {}`.

### G1 peripheral driver priority (cells/drivers/)

| Priority | Driver | Bus/IP | QEMU target | Real-board target |
|----------|--------|--------|-------------|------------------|
| P0 | GPIO cell | PL061 | `0x0903_0000` | BCM2711 GPIO / JH7110 GPIO |
| P0 | UART configure | PL011 | `0x0900_0000` | same (extend serial cell) |
| P1 | I2C cell | I2C master | _(v2 — need board)_ | BCM I2C / DW I2C |
| P1 | SPI cell | SPI master | _(v2)_ | BCM SPI / JH7110 SPI |
| P2 | PWM cell | PWM timer | _(v2)_ | BCM PWM |
| P2 | ADC cell | SPI-ADC / ADS1x | _(v2)_ | external ADC |
| P3 | CAN cell | CAN controller | _(defer)_ | MCP2515 via SPI |

### G2 server driver priority (strict order — each prerequisite for next)

```
1. PCIe ECAM host controller   — port Redox OS enum logic, adapt MmioRegion layer
2. RISC-V IOMMU (2023 spec)    — MANDATORY before NIC (SAS DMA safety)
3. NVMe driver (~3-5K LOC)     — real storage, replaces VirtIO block
4. RTL8125 / Intel i225 2.5G   — real NIC (~5-8K LOC), replaces VirtIO net
5. Intel i40e 10G              — only when inference server needs bandwidth
```

> Not planned: Mellanox mlx5 (100K+ LOC), Bluetooth/WiFi, USB xHCI before G2, full ACPI, audio.

---

## 8. G2: PCIe ECAM + RISC-V IOMMU Strategy

### PCIe ECAM host controller

**Approach:** Port Redox OS [`pci` crate](https://gitlab.redox-os.org/redox-os/drivers) BAR enumeration / capability parsing logic (~40-60% reuse). Rewrite MMIO access layer:

| Layer | Redox approach | Cellos adaptation |
|-------|---------------|------------------|
| MMIO access | Raw pointer cast | `ostd::mmio::MmioRegion` via Resource Registry |
| Driver isolation | Redox userspace process | Cellos Driver Cell (`#![forbid(unsafe_code)]`) |
| BAR mapping | `mmap()` syscall | `request_mmio(base, len)` → `MmioRegion` grant |
| Interrupt | Redox IRQ system | PLIC/GIC async waker dispatch (§3 above) |

Result: PCIe enumeration logic reused, Cellos safety invariants preserved.

### RISC-V IOMMU (non-optional before NIC)

In SAS, a NIC performing DMA without IOMMU control can write to **any** physical address — including kernel TCB/stack. This is a critical security invariant:

- Implement the [RISC-V IOMMU spec (ratified 2023)](https://github.com/riscv-non-isa/riscv-iommu) before installing NIC driver.
- IOMMU maps NIC's DMA window to per-Cell physical regions only.
- `request_mmio` registry extended: DMA range declared → kernel programs IOMMU page tables.

---

## 9. G3: NPU Driver Path

Two-implementation strategy validates `ViAccelerator` trait generality.

### Level A — Tier 1b FFI (G2 work, no kernel change)

- **RKNN Runtime FFI cell** (ARM64, RK3588): Tier 1b C wrapper calls `rknn_init/run/outputs_get`.
- Validates real-world `load_model / submit / wait` semantics before freezing trait contract.
- Prerequisite: Tier 1b entropy shim + net shim (see [05-application.md §3](05-application.md)).

### Level B — Kernel NPU scheduler (G3)

Design `ViAccelerator` trait only after ≥2 months hands-on with RKNN API on real RK3588 hardware. Trait contract must be hardware-informed, not speculative.

```rust
// Draft only — validate against real RKNN API before freezing
pub trait ViAccelerator {
    fn load_model(&mut self, data: Box<[u8]>) -> ViResult<ModelHandle>;
    fn submit(&self, handle: &ModelHandle, input: TensorBuffer) -> ViResult<JobId>;
    fn wait(&self, job: JobId) -> ViResult<TensorBuffer>;
    fn capabilities(&self) -> AcceleratorCaps;
}
```

### Level B+ — SiFive X390 VCIX (second impl)

Port X390 VCIX driver cell when hardware available (Q2 2026). If both RKNN and X390 fit `ViAccelerator` without changes → trait is correctly abstracted.

### Level C — Zero-copy tensor pipeline (G3, after sys_grant_pages)

`sys_grant_tensor` + `TensorBuffer` dual-domain (CpuRam/NpuDram) — prerequisite: `sys_grant_pages` large-buffer IPC (G2 extension). Do not design before Level B is validated.