# Research: Rust Embedded Ecosystem — Trạng thái 2025–2026

**Version**: 1.0
**Last Updated**: 2026-06-11
**Phương pháp**: 46 tool calls, 10+ sources fetched, 7 claims adversarially verified
**Report đầy đủ**: `.agents/reports/research-260611-rust-embedded-2025.md`

> Gap được ghi nhận trong `research-market-survey.md`: "Rust adoption numbers trong embedded KHÔNG có trong confirmed claims."
> File này fill gap đó với data từ 2025–2026.

---

## Tóm tắt executive

| Finding | Verdict |
|---|---|
| Rust embedded adoption % | ~5–15% penetration (floor estimate — no hard survey number) |
| Embassy production status | Production-deployed, no 1.0 stability guarantee |
| RTIC vs Embassy RT benchmark | RTIC 3× faster (1.18µs vs 3.74µs), 38% smaller binary |
| Hubris (Oxide Computer) | Production OS trên rack servers từ July 2023 |
| Ferrocene safety cert | ASIL-D compiler qualified; RISC-V **chưa được qualified** |
| Cellos competitive position | SAS niche **hoàn toàn chưa có đối thủ** trong Rust OS ecosystem |

---

## Findings đã xác nhận

### F1 — Rust embedded adoption: không có hard % nhưng xu hướng rõ ràng
**Confidence: MEDIUM** | Multiple sources, no direct survey breakout

State of Rust 2024 (7,310 respondents): "slight uptick in users targeting embedded and mobile platforms" — không có số cụ thể cho embedded-only. Eclipse Foundation 2024 IoT/Embedded Survey không đo Rust riêng.

**Floor estimate (triangulated)**: Embassy 9,400 GitHub stars + RTIC ~2,900 + probe-rs activity → **~10–30K developers globally**, chiếm ~5–15% addressable embedded developer population (vs 100K+ trong Eclipse surveys).

Automotive là domain tăng mạnh nhất single-year (2024) — được thêm làm closed-answer option lần đầu tiên.

**Cellos implication**: Market nhỏ nhưng growing. "Early mover" trong Rust embedded OS là viable strategy nếu execution tốt.

**Sources**: [State of Rust 2024](https://blog.rust-lang.org/2025/02/13/2024-State-Of-Rust-Survey-results/) · [onevariable.com 2025 production survey](https://onevariable.com/blog/embedded-rust-production/)

---

### F2 — Embassy: production-deployed trên multiple commercial products, không có 1.0 API stability
**Confidence: HIGH** | Multiple production case studies confirmed

9,400 GitHub stars; nightly-free từ ~2024 (stable Rust ≥1.75). Hỗ trợ STM32, nRF, RP2040/RP2350, ESP32, CH32V (RISC-V).

**Confirmed production users**: Akiles (hotel keys), IJboulevard (Amsterdam Centraal lighting), Kelvin Cozy (smart radiator), SuperCritical Redshift 6 (synthesizer), LUCI (wheelchair accessibility).

**Pain point critical cho Cellos comparison**: Cooperative scheduling — một blocking task freeze toàn bộ system. Không phải compile error, không có runtime trap. Embassy là firmware framework, không có VFS/IPC/capability model.

**Sources**: [Embassy GitHub](https://github.com/embassy-rs/embassy) · [onevariable.com production survey](https://onevariable.com/blog/embedded-rust-production/)

---

### F3 — RTIC 2.2.0: fastest interrupt latency, smallest binary cho hard-RT
**Confidence: HIGH** | Independent benchmark by Tweede Golf

RTIC formal basis: Stack Resource Policy (SRP) — deadlock-freedom + priority-inversion-freedom proven tại compile time.

**Benchmark (STM32F446 @ 180MHz, oscilloscope, 200 samples):**

| Metric | RTIC | Embassy | FreeRTOS/C |
|---|---|---|---|
| Interrupt latency | **1.184 µs** | 3.738 µs | 4.973 µs |
| Binary size | **8,888 B** | 14,272 B | 20,676 B |
| Static RAM | **392 B** | 872 B | 5,480 B |

RTIC là scheduling framework only — không có HALs, networking, USB. Thường dùng kết hợp RTIC scheduling + Embassy HALs.

**Cellos implication**: RTIC là ground truth cho hard-RT firmware benchmark. Cellos cần so sánh RT Cell latency với RTIC baseline.

**Sources**: [Tweede Golf benchmark](https://tweedegolf.nl/en/blog/65/async-rust-vs-rtos-showdown) · [RTIC docs.rs](https://docs.rs/crate/rtic/latest)

---

### F4 — Hubris (Oxide Computer): pure-Rust embedded OS, production từ July 2023
**Confidence: HIGH** | Multiple independent sources

3,500 GitHub stars. Deployed làm BMC/service processor trong Oxide rack servers (32-sled AMD racks). First customer: Idaho National Laboratory (DoE).

**Architecture — đối lập hoàn toàn với Cellos**:
- Memory isolation: **MPU hardware** (không phải language safety) ON TOP OF Rust
- Tasks: **static tại build time** — không có dynamic task creation/destruction
- Alloc: **không có dynamic allocation**
- Kernel: ~2,000 lines Rust

Hubris là existence proof rằng production Rust embedded OS khả thi — nhưng niche của nó (MCU security controller, static workload) không overlap với Cellos G1 (robot, dynamic Cells, full stack).

**Sources**: [Hubris site](https://hubris.oxide.computer/) · [Oxide blog](https://oxide.computer/blog/hubris-and-humility) · [The New Stack](https://thenewstack.io/in-pursuit-of-a-superior-server-oxide-computer-ships-its-first-rack/)

---

### F5 — Ferrocene: ASIL-D compiler qualified, nhưng RISC-V chưa là qualified target
**Confidence: HIGH** | Official Ferrocene documentation

Ferrocene 26.02.0: TÜV SÜD qualified — ISO 26262 ASIL-D (compiler), IEC 61508 SIL 3, IEC 62304 Class C (medical).

December 2025: Ferrocene 25.11.0 — first qualified `core` library: IEC 61508 SIL 2 + ISO 26262 ASIL-B trên **Armv7E-M và Armv8-A**.

**Qualified targets hiện tại**: Armv7E-M (Cortex-M4/M7), Armv8-A bare metal, Linux x86-64, QNX — **RISC-V không có trong list**.

**Safety-Critical Rust Consortium** (June 2024): Arm, AdaCore, Ferrous Systems, HighTec, Woven by Toyota — charter chưa có committed deliverables.

**Cellos implication — quan trọng**: Safety market positioning cho RISC-V bị block **ít nhất 12–24 tháng**. Cellos không thể claim ASIL-B path trên RISC-V ngay bây giờ. ARM64 G2 target có path sớm hơn.

**Sources**: [Ferrocene](https://ferrocene.dev/) · [Ferrous libcore news](https://ferrous-systems.com/blog/ferrocene-libcore-news-release/) · [Rust in Safety-Critical Jan 2026](https://blog.rust-lang.org/2026/01/14/what-does-it-take-to-ship-rust-in-safety-critical/)

---

### F6 — Rust embedded pain points: ecosystem gaps > language issues
**Confidence: HIGH** | ACM CCS 2024 (225 developers), multiple sources

Top pain points (ACM CCS 2024):
1. Thiếu MCU support (driver chưa có)
2. C codebase integration friction
3. Certification constraints
4. 2,692 embedded Rust crates fail SAST tools (toolchain config incompatibility)
5. `no_std` porting: "not easy — requires semantic refactoring"
6. Embassy async footgun: one blocking task = system freeze
7. High-criticality: không có MATLAB/Simulink codegen, không có AUTOSAR Classic Rust RTOS
8. 29% developers unconvinced của Rust security benefits (vì firmware avoid heap → ownership ít impactful)

**Cellos implication**: Law 4 (`#![forbid(unsafe_code)]` trong Cells) và Tier 1b C FFI pattern (không phải `no_std` porting) là đúng approach cho vendor SDK integration. Embassy blocking-task footgun không có trong Cellos vì kernel preempts.

**Sources**: [ACM CCS 2024](https://dl.acm.org/doi/10.1145/3658644.3690275) · [Rust Safety-Critical blog](https://blog.rust-lang.org/2026/01/14/what-does-it-take-to-ship-rust-in-safety-critical/)

---

### F7 — Cellos's SAS niche hoàn toàn chưa có đối thủ trong Rust OS landscape
**Confidence: HIGH** | Systematic landscape survey

| Project | Stars | Production | Full Stack | SAS | Dynamic |
|---|---|---|---|---|---|
| Hubris | 3.5K | ✅ Oxide | ❌ | ❌ MPU | ❌ static |
| Embassy | 9.4K | ✅ multiple | ❌ firmware | ❌ | ❌ |
| RTIC | 2.9K | ✅ broad | ❌ scheduling | ❌ | ❌ |
| Tock | ~5K | ⚠️ OpenTitan | ❌ | ❌ | ❌ |
| Ariel OS | <500 | ❌ research | ❌ | ❌ | ⚠️ |
| Theseus | 2.8K | ❌ research | ⚠️ | ✅ | ✅ |
| **Cellos** | early | ❌ WIP | ✅ | ✅ | ✅ |

**Ariel OS** (arxiv 2504.19662, June 2025): closest threat — first Rust embedded OS với single+multicore preemptive + async. Overhead 9.6% (nRF52840). ARM + RISC-V + Xtensa. Tuy nhiên: architecture paper, không có full-stack services.

**Sources**: [Ariel OS](https://arxiv.org/abs/2504.19662) · [Theseus](https://github.com/theseus-os/Theseus)

---

## Comparison Matrix: Cellos vs Embedded Rust Ecosystem

| Dimension | Cellos | Embassy | RTIC 2.x | Hubris |
|---|---|---|---|---|
| Architecture | SAS + LBI | Library framework | Scheduling only | Microkernel + MPU |
| Async support | Full (kernel preemptive) | Full (cooperative) | Partial | None |
| Full stack (VFS/net/GFX) | ✅ | ❌ | ❌ | ❌ |
| Dynamic tasks | ✅ | ✅ | ❌ static | ❌ static |
| RISC-V support | Primary | CH32V (limited) | Yes | Limited |
| Safety cert | ❌ (RISC-V unqualified) | ❌ | ❌ | ❌ |
| Production evidence | ❌ WIP | ✅ multiple | ✅ broad | ✅ Oxide |
| Memory isolation | Language (SAS) | None | None | MPU hardware |
| OTA hot-swap | ✅ | ❌ | ❌ | ❌ |
| Fault recovery (no reboot) | ✅ | ❌ | ❌ | ❌ |

---

## Chiến lược positioning cho Cellos

**① Position là "Embassy + OS" — rung missing giữa bare-metal và full OS**
Embassy/RTIC không có VFS, IPC, capability model, compositor. Cellos là Rust-native OS duy nhất với full service stack cho embedded/robot. Pitch: *"You outgrow Embassy when you need filesystem, networking, UI, and composable services — that's when Cellos starts."*

**② Leverage Ferrocene RISC-V timeline asymmetry**
RISC-V safety cert target: 12–24 tháng nữa. Khi Ferrocene thêm RISC-V vào qualified list, Cellos là Rust OS duy nhất đã có full RISC-V story sẵn. Track announcement làm market trigger.

**③ Treat Embassy/RTIC là ecosystem, không phải competition**
Cellos Cells có thể host Embassy HALs làm drivers. Framing: "OS orchestration layer above Embassy-style drivers." Neutralize "why not just use Embassy" objection.

---

## Caveats

- Embedded WG Micro-Survey 2024 results chưa publish khi research chạy — khi có sẽ là source tốt nhất cho framework-level % adoption
- 5–15% penetration estimate = triangulated from GitHub stars, không phải direct measurement
- RISC-V Ferrocene target timeline chưa được publicly committed
- Không có commercial revenue data cho bất kỳ Rust embedded OS nào

---

*Sources: [State of Rust 2024](https://blog.rust-lang.org/2025/02/13/2024-State-Of-Rust-Survey-results/) · [Embassy](https://github.com/embassy-rs/embassy) · [RTIC](https://rtic.rs/) · [Hubris](https://hubris.oxide.computer/) · [Ferrocene](https://ferrocene.dev/) · [Safety-Critical Rust Jan 2026](https://blog.rust-lang.org/2026/01/14/what-does-it-take-to-ship-rust-in-safety-critical/) · [Ariel OS](https://arxiv.org/abs/2504.19662) · [Tweede Golf benchmark](https://tweedegolf.nl/en/blog/65/async-rust-vs-rtos-showdown) · [ACM CCS 2024](https://dl.acm.org/doi/10.1145/3658644.3690275) · [onevariable production](https://onevariable.com/blog/embedded-rust-production/)*
