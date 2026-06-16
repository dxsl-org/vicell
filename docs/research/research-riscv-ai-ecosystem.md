# Research: RISC-V AI Accelerator Ecosystem — 2025–2026

**Version**: 1.0
**Last Updated**: 2026-06-11
**Phương pháp**: 41 tool calls, 16 sources fetched, 7 findings verified
**Report đầy đủ**: `.agents/reports/research-260611-riscv-ai-ecosystem.md`

> ⚠️ **Research này correct một số assumptions quan trọng trong G2 strategy hiện tại.**
> Đọc phần "Corrections" trước khi update roadmap.

---

## Tóm tắt executive

| Claim trong G2 strategy | Thực tế | Severity |
|---|---|---|
| "C930 target G2" | C930 = IP core; không có SoC/board trước 2027 | 🔴 HIGH |
| "P870+X390 (Q2 2026)" | P870-D là SiFive IP; không có board mua được | 🔴 HIGH |
| "RKNN SDK trên K230" | K230 dùng **nncase** (Canaan); RKNN = Rockchip ARM-only | 🔴 HIGH |
| "Tier 3b Linux VM trên RISC-V" | H-ext chưa có trong BẤT KỲ chip RISC-V shipping nào | 🔴 HIGH |
| "12-18 month technical window" | Window là geopolitical (China export controls), không phải technical | 🟡 MEDIUM |

**Hardware path thực tế cho G2 development:**
1. **Milk-V Pioneer (SG2042)** — ~$600, 64-core, purchasable NOW
2. **CanMV K230D** — $29, 6 TOPS NPU, nncase v2.10 mature
3. **BPI-F3 (SpacemiT K1)** — ~$100, RVV 1.0 + measured llama.cpp

---

## Findings đã xác nhận

### F1 — Alibaba C930: IP core, không phải chip
**Confidence: HIGH** | The Register + Tom's Hardware (Tier 1)

"Ships this month" (March 2025) = RTL delivery cho IP licensees — không khác gì ARM Cortex IP license. Không có SoC product, không có dev board trước 2027.

Không có specs chính thức nào được publish: không có core count, clock speed, cache size. Claim "512b VLEN + 8 TOPS matrix engine" chỉ xuất hiện trên một Chinese secondary source — LOW confidence.

H-ext (CoVE): được list trong C930 extension nhưng shipping status chưa confirmed.

**ViCell implication**: Loại C930 khỏi G2 hardware roadmap gần hạn. Cân nhắc làm G2 target xa hạn (2027+) nếu SoC cuối cùng xuất hiện với H-ext.

**Sources**: [The Register](https://www.theregister.com/2025/03/05/china_alibaba_risc_v_c930/) · [Tom's Hardware](https://www.tomshardware.com/pc-components/cpus/alibaba-launches-risc-v-based-xuantie-c930-server-cpu-ai-hpc-chip-ships-this-month-more-designs-to-follow)

---

### F2 — "Sophgo P870" không tồn tại; target thực là SG2042/SG2044
**Confidence: HIGH** | SiFive official + Milk-V community

**P870-D = SiFive core IP** (RVA23, up to 256 cores per SoC). Sophgo licensed **SiFive P670** (không phải P870) làm SG2380 — và SG2380/Milk-V Oasis vẫn là vaporware (pre-orders open, hardware chưa tồn tại).

**Chip thực sự đang ship:**
- **SG2042**: 64-core RV64GC @ 2GHz, DDR4-3200×4, PCIe Gen4×32; **không có RVV 1.0** (chỉ pre-ratification V ext); Pioneer Box ~$600 — **có thể mua ngay**
- **SG2044**: RVV 1.0, DDR5, 32 memory controllers (8× SG2042 bandwidth), 4.91× STREAM; launched Feb 2025; "world's first RISC-V server deeply adapted to DeepSeek"

**Sources**: [SiFive P870-D](https://www.sifive.com/cores/performance-p800) · [TechPowerUp SG2044](https://www.techpowerup.com/333496/sophgo-unveils-new-products-at-the-2025-china-risc-v-ecosystem-conference) · [ScienceDirect SG2042 LLM](https://www.sciencedirect.com/science/article/abs/pii/S0167739X25005369)

---

### F3 — Kendryte K230D: $29, 6 TOPS, nncase v2.10 mature — KHÔNG DÙNG RKNN
**Confidence: HIGH** | CNX-Software + kendryte/nncase GitHub

Dual-core C908 (RVV 1.0, VLEN=128b), 6 TOPS INT8 KPU, 128MB embedded LPDDR4.
- ResNet-50 ≥85fps, YoloV5S ≥38fps (confirmed across multiple board vendors)
- **nncase v2.10** (Aug 2025): supports ONNX/TFLite/Caffe, C++ runtime stable, kmodel format portable

**⚠️ CRITICAL CORRECTION**: RKNN = Rockchip NPU SDK cho RK3588 ARM chips. K230 dùng **nncase** (Canaan's own framework). Đây là hai ecosystems hoàn toàn khác nhau.

KPU operates on physical addresses → compatible với ViCell grant-based DMA architecture về nguyên tắc (cần hands-on validation).

**ViCell implication**: K230D là hardware lý tưởng để validate G3 ViAccelerator API design với chi phí thấp nhất. Sửa tất cả references "RKNN trên K230" trong docs.

**Sources**: [CNX-Software K230D](https://www.cnx-software.com/2024/11/18/29-banana-pi-bpi-canmv-k230d-zero-features-kendryte-k230d-risc-v-soc-for-aiot-applications/) · [kendryte/nncase](https://github.com/kendryte/nncase/releases)

---

### F4 — SpacemiT K1 (BPI-F3): RVV 1.0 board duy nhất có measured llama.cpp
**Confidence: HIGH** | Independent benchmark published

8-core X60 @ 1.6GHz, VLEN=256b, 4 Integer Matrix Engines. Measured (không phải vendor claim):
- Llama 3.2 1B (Q4X): **8.64 t/s prefill / 5.29 t/s decode** với RVV vectorized kernels
- vs scalar: 2.93/2.38 t/s — **~3× RVV speedup confirmed**

BPI-F3 available ~$100; Milk-V Jupiter (same SoC) sold out trong 3 tháng.

**ViCell implication**: BPI-F3 là cheapest way để validate ViCell vector IPC performance trên real RVV 1.0 hardware. Llama inference latency là good proxy cho ViCell RT Cell scheduling overhead.

**Sources**: [10xEngineers benchmark](https://10xengineers.ai/llm-inference-with-codebook-based-q4x-quantization-using-the-llama-cpp-framework-on-risc-v-vector-cpus/) · [RVV benchmark site](https://camel-cdr.github.io/rvv-bench-results/bpi_f3/index.html)

---

### F5 — H-ext (Hypervisor extension): KHÔNG có trong BẤT KỲ chip shipping nào — blocks Tier 3b
**Confidence: HIGH** | Systematic search + arXiv RISC-V production paper

RISC-V H-ext là ratified spec nhưng chưa có commercial RISC-V chip nào implement và ship. Đây là hard blocker cho ViCell G2's Tier 3b Linux VM management plane trên RISC-V.

**Software ecosystem gaps measured** (SG2042 production deployment):
- CoreMark: 2.7× slower than Intel Xeon
- Disk sequential: 7× slower; disk random: **45× slower**
- Network: 10× slower
- PyTorch/ONNX Runtime: compile được nhưng không có official PyPI wheel; no production-ready distribution
- IREE microkernels: RISC-V RVV backend explicitly missing (effort bắt đầu 2025)

**Workaround cho H-ext gap**:
- Option A: Tier 3b trên separate ARM64/x86 management node (thực tế nhất hiện tại)
- Option B: Defer Tier 3b đến khi H-ext ship (SG2044 successor, 2026-2027?)

**ViCell implication**: Redesign G2 two-plane architecture — data plane RISC-V native; management plane cần separate node hoặc được defer.

**Sources**: [arXiv RISC-V production 2505.02650](https://arxiv.org/html/2505.02650) · [IREE RISC-V paper 2508.14899](https://arxiv.org/html/2508.14899)

---

### F6 — Software ecosystem: gaps đo được, không chỉ anecdotal
**Confidence: HIGH** | Multiple peer-reviewed sources

| Gap | Status | Severity cho ViCell |
|---|---|---|
| GCC/LLVM RISC-V backend | ✅ Mature | Not blocking |
| RVV 1.0 codegen | ✅ Works | Not blocking |
| PyTorch RISC-V | ⚠️ Compilable, no wheel | G2 medium-term |
| ONNX Runtime RISC-V | ⚠️ UCB fork, not production | G2 medium-term |
| IREE RVV microkernels | ❌ Explicitly missing | G3 AI kernel needed |
| H-ext | ❌ No shipping chip | Tier 3b blocked |
| Linux BSP quality | ✅ Good (SG2042) | Not blocking |

---

### F7 — "12-18 month window" là geopolitical, không phải technical
**Confidence: MEDIUM** | Multiple secondary + RISC-V Annual Report 2025

Window driven bởi US export controls → China không mua được NVIDIA H100/H200 → demand cho RISC-V AI chips (SG2044 + DeepSeek integration = concrete evidence).

**Threat landscape:**
- Arm forecast: 90% của custom AI server chips by 2029 → RISC-V bị exclude
- SiFive X100 Gen 2: 64 TFLOPS FP8, Q2 2026 first silicon → RISC-V AI inference trên Linux viable cho generic workloads
- Ventana Veyron V2 (acquired by Qualcomm Dec 2025) → mainstream RISC-V server cores sắp đến hyperscalers

**ViCell's real moat** (không thay đổi sau khi Linux catches up): Not "RISC-V OS" mà **"zero-copy OS-level pipeline với guaranteed P99 latency bounds."** Linux + managed runtimes (PyTorch + ONNX) unlikely deliver comparable P99 latency trước 2027.

**Sources**: [Tom's Hardware ARM 90%](https://www.tomshardware.com/pc-components/cpus/report-claims-arm-chips-will-power-90-percent-of-ai-servers-based-on-custom-processors-in-2029-x86-and-risc-v-on-the-outside-looking-in) · [RISC-V Annual Report 2025](https://riscv.org/wp-content/uploads/2026/01/RISC-V-Annual-Report-2025.pdf)

---

## Hardware Roadmap Thực tế cho ViCell G2

| Phase | Board | Price | Purpose | Status |
|---|---|---|---|---|
| **Now (G1 NPU validation)** | CanMV K230D | $29 | ViAccelerator API design + nncase integration | ✅ Available |
| **Now (RVV benchmark)** | BPI-F3 (SpacemiT K1) | ~$100 | RT Cell + vector IPC benchmark | ✅ Available |
| **G2 development** | Milk-V Pioneer (SG2042) | ~$600 | 64-core RISC-V server, Linux BSP mature | ✅ Available |
| **G2 demo (future)** | SG2044 SRA3-40 | TBD | RVV 1.0 + DDR5, DeepSeek inference | 🟡 Wait |
| **Long term** | C930 SoC (unnamed) | TBD | IF H-ext ships, premium G2 target | ❌ 2027+ |

**Do NOT plan around**: SG2380/Milk-V Oasis (vaporware), C930 SoC (2027+), any "P870 chip".

---

## Caveats

- Không có hands-on benchmarks; tất cả TOPS/t/s claims là vendor-stated hoặc third-party academic
- SG2044 memory bandwidth (4.91×) chưa được independently verified
- H-ext gap cần dedicated confirmation từ upcoming SoCs (Sophgo roadmap)
- China-domestic commercial dynamics (procurement, Kylin/UOS cert) không được assess

---

*Sources: [The Register C930](https://www.theregister.com/2025/03/05/china_alibaba_risc_v_c930/) · [TechPowerUp SG2044](https://www.techpowerup.com/333496/sophgo-unveils-new-products-at-the-2025-china-risc-v-ecosystem-conference) · [ScienceDirect SG2042 LLM](https://www.sciencedirect.com/science/article/abs/pii/S0167739X25005369) · [arXiv SG2044 SC'25](https://arxiv.org/abs/2508.13840) · [CNX-Software K230D](https://www.cnx-software.com/2024/11/18/29-banana-pi-bpi-canmv-k230d-zero-features-kendryte-k230d-risc-v-soc-for-aiot-applications/) · [nncase GitHub](https://github.com/kendryte/nncase/releases) · [10xEngineers benchmark](https://10xengineers.ai/llm-inference-with-codebook-based-q4x-quantization-using-the-llama-cpp-framework-on-risc-v-vector-cpus/) · [arXiv RISC-V production 2505.02650](https://arxiv.org/html/2505.02650) · [RISC-V Annual Report 2025](https://riscv.org/wp-content/uploads/2026/01/RISC-V-Annual-Report-2025.pdf)*
