# Research: AUTOSAR Classic vs Adaptive — OS Requirements và Pain Points

**Nguồn**: Deep research — 64 agents, 6 sources fetched, 15 claims verified (4 confirmed / 11 killed)  
**Venues**: AUTOSAR R22-11/R24-11 specs (autosar.org), arXiv:1912.01367, BTC Embedded Systems AG (AUTOSAR consortium member), MathWorks, Wind River  
**Date**: 2026-06-08

---

## Tóm tắt executive

AUTOSAR tồn tại dưới hai nhánh có mục tiêu đối lập: **Classic** (static, hard real-time, ASIL-D) và **Adaptive** (dynamic SOA, OTA, ASIL-B max). 4 findings được xác nhận mô tả cấu trúc phân tách này và workflow pain points của Adaptive. Research bị giới hạn bởi session rate limit — nhiều developer pain points cụ thể (C-only tooling, no memory safety) không được xác nhận từ nguồn primary đủ chất lượng.

**ViCell positioning**: ViCell gần với Adaptive về dynamism nhưng không có POSIX overhead. Để cạnh tranh trong automotive/industrial, ViCell cần: static schedulability analysis mode, SOME/IP/DDS compatibility, ASIL-B toolchain path.

---

## Findings đã xác nhận (adversarially verified)

### F1 — Classic vs Adaptive: đối lập kiến trúc cơ bản
**Confidence: HIGH** | Vote: 3-0 (2 claims độc lập, merged)

AUTOSAR Classic cung cấp hard real-time, ASIL-D scheduling qua static compile-time configuration — tất cả task graphs, data flow và communication paths cố định tại build time. AUTOSAR Adaptive giải quyết inflexibility của Classic bằng cách enable dynamic service registration, OTA updates và C++ development trên POSIX-capable ECUs — nhưng đánh đổi hard real-time guarantees; Adaptive chỉ phù hợp soft real-time domains (ADAS, connected services) với ceiling ASIL-B.

**Sources**:
- AUTOSAR R24-11 Classic Platform OS spec (autosar.org)
- arXiv:1912.01367 "Achieving Determinism in Adaptive AUTOSAR" (2020)
- MathWorks AUTOSAR platform comparison (mathworks.com/help/autosar/ug/autosar-platform-comparison.html)

---

### F2 — Classic: static binding không thể reconfigure runtime
**Confidence: HIGH** | Vote: 3-0

AUTOSAR Classic tightly binds software vào ECU configurations tại design time. Task schedules, data flow topology và RTE communication paths đều được code-generate tại compile time và không thể reconfigure tại runtime. AUTOSAR Methodology định nghĩa 3 configuration classes (pre-compile, link-time, post-build) nhưng chỉ parameter values (calibration data) mới có thể vary post-build; structural schedule và data-flow graph không thể thay đổi mà không regenerate và recompile toàn bộ RTE.

**Sources**:
- AUTOSAR Methodology spec và RTE spec (autosar.org, R22-11)
- AUTOSAR Layered Software Architecture spec

**Note từ verifier**: Post-build configuration cho phép calibration parameters vary per vehicle variant mà không recompile — claim về parameter inflexibility là overstated. Structural binding claim (schedules, data flow) là chính xác.

---

### F3 — Adaptive: OTA updates, dynamic service discovery qua SOME/IP-SD và DDS
**Confidence: HIGH** | Vote: 3-0

AUTOSAR Adaptive Platform enable OTA updates qua UCM (Update and Configuration Management) functional cluster, dynamic service registration qua ara::com với SOME/IP-SD over UDP. Service interfaces được specify trong ARXML ServiceInterface elements và compile thành language-binding proxies/skeletons. DDS là alternative ara::com binding theo ARAComAPI spec.

**Key constraint**: Interfaces phải được pre-compiled từ ARXML — ad-hoc runtime interface creation không được support. "Dynamic" áp dụng cho discovery và binding, không phải interface definition.

**Sources**:
- AUTOSAR AP UCM spec (SWS PDFs, R17-10 through R25-11)
- AUTOSAR EXP ARAComAPI (R22-11, R23-11)
- AUTOSAR Foundation SOME/IP-SD Protocol Specification (FO R18-10 through R22-11)

---

### F4 — Adaptive: significant workflow shift cho automotive developers
**Confidence: MEDIUM** | Vote: 3-0

Adopting AUTOSAR Adaptive đòi hỏi workflow shift đáng kể so với Classic: engineers phải quản lý runtime service coordination, runtime diagnostics, và runtime variability — những thứ mà Classic's static integration model xử lý tại build time. Classic giải phóng developer khỏi runtime concerns; Adaptive đòi hỏi thinking about service discovery, runtime configuration, và OTA safety.

**Sources**: BTC Embedded Systems AG blog (AUTOSAR consortium member), Wind River, ijeret.org academic paper

**Confidence medium** vì primary source là vendor blog, dù technical content chính xác và multi-source confirmed.

---

## Mapping sang ViCell

| AUTOSAR Requirement | ViCell Status | Gap |
|---|---|---|
| Dynamic service registration | ✅ RegisterService/LookupService (205/206) | Cần SOME/IP-SD compatibility layer |
| OTA update (UCM equivalent) | ✅ Cell hot-swap với StateTransfer | Cần ARXML/UCM-compatible manifest format |
| POSIX-capable runtime | ❌ ViCell là non-POSIX | Tier 3b Linux VM nếu cần POSIX |
| Hard real-time (ASIL-D) | ⚠️ RT watchdog + preemptive scheduler | Chưa có WCET analysis tool, ASIL-D cert path |
| C++ development | ❌ Rust-only Cells | Tier 1b có thể wrap C++ vendor libs |
| Static configuration mode | ❌ ViCell luôn dynamic | Cần static-schedulable subset cho automotive use |

**Verdict**: ViCell competitive với Adaptive (dynamic, modern language, OTA) nhưng không thể thay Classic trong ASIL-D powertrain/braking. Target market: ADAS domain controllers, connected ECUs — nơi Adaptive đang grow nhưng POSIX overhead là vấn đề.

---

## Open Questions

1. AUTOSAR Adaptive R25-11 có introduce native hard real-time scheduling (cyclic execution, deadline monotonic) không?
2. ASIL-D production vehicles có dùng Adaptive cho braking/steering không, hay chỉ ASIL-B ADAS?
3. Minimum AUTOSAR Classic interfaces (COM stack, RTE API, DCM) cần thiết để được Tier-1 supplier acceptance?
4. ViCell RT watchdog + Rust async runtime có đủ static analyzability cho ASIL-B safety case không?

---

## Caveats

- **Source quality**: 4 confirmed claims đều trace về BTC Embedded Systems AG vendor blog, corroborated bởi AUTOSAR primary specs. Không có independent peer-reviewed study so sánh trực tiếp.
- **Refuted claims**: 11 claims bị kill (0-3 hoặc 1-2), bao gồm nhiều quantitative claims về jitter (20ms), ASIL decomposition complexity, và hybrid testbed performance. Surviving claims là architectural-level, không quantitative.
- **Developer pain points gap**: C-only tooling, no memory safety, no hot update — những complaints cụ thể mà devs có — KHÔNG có trong confirmed set. Đây là gap lớn nhất của research này.
- **Time sensitivity**: AUTOSAR Adaptive đang active development (R25-11 released Nov 2025). Gaps có thể đã được address.

---

*Sources: [AUTOSAR autosar.org](https://www.autosar.org/) · [arXiv:1912.01367](https://arxiv.org/abs/1912.01367) · [BTC Embedded AUTOSAR comparison](https://www.btc-embedded.com/autosar-classic-vs-adaptive/) · [MathWorks AUTOSAR](https://www.mathworks.com/help/autosar/ug/autosar-platform-comparison.html)*
