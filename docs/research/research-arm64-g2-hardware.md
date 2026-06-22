# Research: ARM64 G2 Near-Term Hardware — RK3588 + Alternatives

**Version**: 1.0
**Last Updated**: 2026-06-11
**Phương pháp**: 29 tool calls, 15+ sources fetched, 8 findings adversarially verified
**Report đầy đủ**: `.agents/reports/research-260611-arm64-g2-hardware.md`

> ⚠️ **Research này đảo ngược assumption trong G2 strategy:** ARM64 RK3588 là G2 graduation demo target thực tế — không phải RISC-V (C930/P870 bị delay, H-ext chưa tồn tại).

---

## Tóm tắt executive — Khuyến nghị rõ ràng

**Mua: Radxa ROCK 5B+ 16GB (~$149) làm G2 ARM64 graduation demo board.**

Lý do:
1. RK3588 NPU có thể mua ngay, RKNN SDK v2.3 mature — không có RISC-V NPU nào ship trước 2027
2. Cellos đã chạy ARM64 (QEMU virt ring-3 smoke tests passing) — porting RK3588 chỉ cần U-Boot handoff + UART/timer HAL
3. **Tier 3b Linux VM là BÉT HƠN trên ARM64 so với RISC-V** — KVM trên RK3588 EL2 đã confirmed (Alpine Linux), RISC-V H-ext chưa tồn tại trên bất kỳ chip nào
4. Cellos sẽ là **OS đầu tiên có RKNN NPU access trên RK3588** (Zephyr chỉ có UART, Redox không có port)

---

## Findings đã xác nhận

### F1 — RK3588 NPU: 6 TOPS INT8 (peak), ~5.2 TOPS (sustained), 10–15 tok/s TinyLlama 1.1B
**Confidence: HIGH** | 3+ independent community benchmarks

4× Cortex-A76 @ 2.4GHz + 4× Cortex-A55 @ 1.8GHz, 3-core NPU @ 2 TOPS each — vendor-stated.

**Independent measurements (không phải vendor claim):**
- ResNet18 INT8: 244 FPS / 4.09ms (Orange Pi 5 Max)
- YOLOv5s INT8: 54+ FPS
- **TinyLlama 1.1B W8A8: 10–15 tok/s** (Rockchip official, corroborated 3+ community reports)
- Power: ~5–6W under AI load → ~49 FPS/W for ResNet18
- FP16: ~0.5 TFLOPS (12× thấp hơn INT8 — sử dụng INT8 là mandatory cho production NPU work)

**Apple M4 comparison** (reference only, not a target): ~38–51 TOPS — ratio ~6–7× hơn RK3588. Cellos's pitch là latency guarantee, không phải throughput parity.

**Sources**: [TinyComputers.io](https://tinycomputers.io/posts/rockchip-rk3588-npu-benchmarks.html) · [IEEKER](https://ieeker.com/rk3588-npu-performance-industrial-edge-ai/) · [clehaxze benchmark](https://clehaxze.tw/gemlog/2024/02-14-benchmarking-rk3588-npu-matrix-multiplcation-performance-ep2.gmi)

---

### F2 — Mainline Linux 6.13: functional cho CPU/GPU/NIC, NPU cần vendor BSP kernel
**Confidence: HIGH** | CNX-Software + Collabora official

**Works in mainline (≤6.13):** CPU freq scaling, Mali-G610 3D (6.10), 2.5GbE NIC (6.7), USB3 (6.10), HDMI (6.13), VP8/H.264/JPEG decode.

**NPU (RKNN):** Đòi hỏi **vendor BSP kernel (5.10 hoặc 6.1)** với RKNPU driver hôm nay. Open-source Teflon driver target Mesa 25.3 / Kernel 6.18 — chưa ship.

**Trade-off**: Vendor kernel = full RKNN + NPU, nhưng 3–4 year security patch lag vs mainline. Development path: start với vendor BSP → migrate khi Teflon upstreams.

**Sources**: [CNX-Software Dec 2024](https://www.cnx-software.com/2024/12/21/rockchip-rk3588-mainline-linux-support-current-status-and-future-work-for-2025/) · [Collabora](https://www.collabora.com/news-and-blog/news-and-events/rockchip-rk3588-upstream-support-progress-future-plans.html)

---

### F3 — SMMU/IOMMU: on-chip peripherals OK, PCIe KHÔNG isolated
**Confidence: HIGH** | Armbian forum + kernel patchwork

RK3588 có 2 MMU600 instances:
- **Platform/PHP SMMU (NPU, GPU)**: ✅ Working — 17 IOMMU groups, Cellos grant-based DMA safety applies
- **PCIe SMMU**: ❌ PCIe devices không được assign IOMMU groups — community-confirmed 2025; VFIO passthrough fails; multi-patch chain chưa fully merged

**Cellos implication**: G2 DMA safety scope rõ ràng:
- NPU inference pipeline: IOMMU-protected ✅
- PCIe NIC (server networking): không isolated ❌ — document explicitly

**Sources**: [Armbian forum](https://forum.armbian.com/topic/28914-orange-pi-5-plus-rk3588-iommu-smmu-vfio-pci-passthrough/) · [Patchwork mmu600_pcie patch Nov 2024](https://patchwork.kernel.org/project/linux-arm-kernel/patch/20241107123732.1160063-2-cassel@kernel.org/)

---

### F4 — RKNN SDK v2.3.2: mature, proprietary binary, bindgen-compatible cho Tier 1b FFI
**Confidence: HIGH** | GitHub + PyPI official

ONNX/PyTorch/TFLite/Caffe → `.rknn` conversion qua Python host (không phải on-device). Inference qua C API: `rknn_init`, `rknn_run`, `rknn_query` — **bindgen-compatible trực tiếp**.

- v2.3.2 (April 2025): 6-month release cadence, active maintenance
- RKNPU kernel driver: **open source** (BSP tree)
- RKNN-Toolkit2 converter: **proprietary binary** (pip/GitHub)
- `librknnrt.so`: free to use; commercial redistribution terms không documented công khai — cần verify trước khi ship

**⚠️ Nhắc lại**: RKNN = Rockchip ARM-only. Incompatible với SG2042/K230/X390.

**Sources**: [RKNN-Toolkit2 GitHub](https://github.com/rockchip-linux/rknn-toolkit2) · [PyPI v2.3.2](https://pypi.org/project/rknn-toolkit2/)

---

### F5 — KVM trên RK3588: Alpine Linux confirmed, 4 vCPU hard limit
**Confidence: HIGH** | Proxmox community + ubuntu-rockchip discussions

KVM EL2 VHE mode hoạt động. Alpine Linux VMs confirmed booting trên OrangePi 5+ dưới Proxmox (pxvirt ARM64).

**Hard constraint**: Không thể mix A76 + A55 — max 4 vCPUs per VM (big-only hoặc little-only cluster).

**No GPU hoặc PCIe NIC passthrough** — bị blocked bởi PCIe IOMMU gap (F3 ở trên).

**Cellos Tier 3b**: Alpine Linux VM (Prometheus/SSH/PostgreSQL) là feasible. 4-vCPU ceiling chấp nhận được cho management traffic.

**Sources**: [Proxmox OrangePi5+](https://codingfield.com/blog/2024-01/install-armbian-and-proxmox-on-orangepi5plus/) · [pxvirt ARM64](https://github.com/jiangcuo/pxvirt)

---

### F6 — Rust ARM64 hypervisor prior art: cloud-hypervisor + 30K LOC no_std bare-metal
**Confidence: HIGH** | GitHub official sources

- `cloud-hypervisor`: AArch64 Tier-1 arch, EL2 VHE mode, SVE/SVE2 guest support
- 30K LOC no_std Rust ARM64 hypervisor (replaces Google Hafnium SPMC): demonstrates EL2 bare-metal Rust feasibility

Cả hai đều là reference architecture — không phải embedded/bare-metal Type-1 không cần Linux host. Nhưng validate rằng **EL2 bare-metal Rust trên ARM64 là feasible** cho Cellos Tier 3 VMM.

**Sources**: [cloud-hypervisor AArch64](https://intelkevinputnam.github.io/cloud-hypervisor-docs-HTML/docs/arm64.html) · [Rust forum no_std ARM64 VMM](https://users.rust-lang.org/t/30k-lines-of-no_std-rust-a-bare-metal-arm64-hypervisor-that-replaces-googles-hafnium-spmc/139497)

---

### F7 — Qualcomm X Elite: không viable cho G2
**Confidence: HIGH** | Official documentation

No developer board cho bare-metal OS work. Hexagon NPU bị blocked sau proprietary QAIRT SDK + Windows-first access. No public bare-metal boot docs cho EL1/EL2. **Abandon cho G2.**

---

### F8 — Competing OS trên RK3588: Cellos sẽ là đầu tiên với RKNN NPU
**Confidence: HIGH** | Zephyr docs + Armbian/ubuntu-rockchip survey

- **Armbian/ubuntu-rockchip**: Ubuntu-Rockchip (vendor BSP 6.1) + RKNN là baseline của community
- **Zephyr**: DTB-only port (Firefly ROC-RK3588-PC); A55 UART/GPIO only; không có RKNN, không có Mali GPU
- **Redox OS**: không có RK3588 port

**Graduation demo claim**: *"Cellos — first custom OS với deterministic NPU inference trên RK3588"* là credible và verifiable.

---

## Hardware Comparison Table

| Board | Price (16GB) | NPU | NVMe | NIC | KVM | Armbian | Ghi chú |
|---|---|---|---|---|---|---|---|
| Orange Pi 5 (8GB) | ~$75 | RKNN (BSP) | M.2 PCIe×4 | 1× 1GbE | ✅ | ✅ | Cheapest entry |
| Orange Pi 5 Plus (16GB) | ~$109 | RKNN (BSP) | M.2 PCIe×4 | 2× 2.5GbE | ✅ | ✅ | Best NIC |
| **Radxa ROCK 5B+ (16GB)** | **~$149** | **RKNN (BSP)** | **2× M.2 PCIe×4** | **1× 2.5GbE** | **✅** | **✅** | **Primary recommendation** |
| SG2042 Pioneer (no NPU) | ~$600 | ❌ | NVMe | 2× 1GbE | ❌ H-ext | ⚠️ | RISC-V latency demo only |

---

## Development Path cho G2 ARM64

```
1. Boot: U-Boot → Cellos EL1 (same path as QEMU virt, FDT từ U-Boot)
2. BSP: Ubuntu-Rockchip vendor kernel 6.1 cho RKNN bring-up
3. Tier 1b: RKNN C API → Rust FFI Cell (librknnrt.so)
4. Tier 3b: Alpine Linux VM qua KVM EL2 (4 vCPU)
5. SMMU scope: document PCIe IOMMU gap; scope DMA safety to on-chip (NPU/GPU)
6. Migrate to mainline 6.18+ khi Teflon NPU driver upstreams
```

**Parallel track**: Giữ SG2042 Pioneer cho RISC-V latency/reliability story (không cần NPU ở đó).

---

## Caveats

- Không có physical hardware tests — NPU numbers là third-party community measurements
- RK3588S vs RK3588 distinction: OrangePi 5 = RK3588S (fewer PCIe lanes); ROCK 5B = full RK3588
- PCIe IOMMU status là moving target (Nov 2024 patch series) — re-check trước khi claim PCIe DMA isolation
- `librknnrt.so` commercial redistribution terms cần verify trước khi ship

---

*Sources: [CNX-Software RK3588 mainline](https://www.cnx-software.com/2024/12/21/rockchip-rk3588-mainline-linux-support-current-status-and-future-work-for-2025/) · [TinyComputers.io NPU benchmark](https://tinycomputers.io/posts/rockchip-rk3588-npu-benchmarks.html) · [RKNN-Toolkit2](https://github.com/rockchip-linux/rknn-toolkit2) · [Armbian IOMMU](https://forum.armbian.com/topic/28914-orange-pi-5-plus-rk3588-iommu-smmu-vfio-pci-passthrough/) · [cloud-hypervisor AArch64](https://intelkevinputnam.github.io/cloud-hypervisor-docs-HTML/docs/arm64.html) · [Proxmox OrangePi5+](https://codingfield.com/blog/2024-01/install-armbian-and-proxmox-on-orangepi5plus/) · [RKNN pxvirt ARM64](https://github.com/jiangcuo/pxvirt)*
