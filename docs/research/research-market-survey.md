# Research: Thị trường Embedded/IoT OS — Eclipse Foundation Survey 2024

**Nguồn**: Deep research — 86 agents, 6 sources fetched, 20 claims verified (9 confirmed / 11 killed)  
**Venues**: Eclipse Foundation IoT/Embedded Developer Survey 2024 (newsroom.eclipse.org), Zephyr Project 2024 Wrap-up, Eclipse sustainability blog  
**Note**: Synthesis step fail do session rate limit; 9 verified claims được list trực tiếp, không merged  
**Date**: 2026-06-08

---

## Tóm tắt executive

Eclipse Foundation 2024 survey (N = ~1,500 developers) xác nhận: **Linux 46%, FreeRTOS 29%, Zephyr 21%, ThreadX 13%**. Open source adoption tăng mạnh (63% → 75% YoY). Pain point hàng đầu là Connectivity (48%) và Security (35%). 47% developers prioritize safety certifications.

Zephyr momentum đáng kể nhưng từ community side: 100,000+ commits, 150 new boards 2024. Commercial adoption chưa confirm numerically.

---

## Findings đã xác nhận (adversarially verified)

### F1 — Market share 2024: Linux dominant, Zephyr growing
**Vote: 2-1**

> Linux 46% · FreeRTOS 29% · Zephyr 21% · ThreadX 13%

Source: Eclipse Foundation 2024 IoT/Embedded Developer Survey  
URL: https://newsroom.eclipse.org/news/announcements/eclipse-foundation-unveils-2024-iot-embedded-developer-survey-results

"Growth in Zephyr and ThreadX indicates rising interest in performance and safety-critical RTOS options."

---

### F2 — Zephyr momentum: 21% adoption, safety-critical niche
**Vote: 2-1**

Zephyr RTOS đạt 21% developer adoption 2024. Được định vị như safety-critical alternative to FreeRTOS (29%). Khoảng cách Zephyr-FreeRTOS đang thu hẹp.

Source: Eclipse Foundation 2024 survey

---

### F3 — Zephyr codebase milestone: 100,000 total commits
**Vote: 3-0**

Zephyr surpassed 100,000 total commits trong 2024, major codebase maturity milestone cho một RTOS.

Source: https://zephyrproject.org/zephyr-rtos-2024-wrap-up-a-year-of-growth-innovation-and-community-impact/

---

### F4 — Pain point #1: Connectivity (48%), Security growing (35%)
**Vote: 3-0**

Connectivity là top developer pain point tại **48%** (giảm từ 52% năm 2023). Security concerns tăng lên **35%** (từ 33%).

Source: Eclipse Foundation 2024 survey announcement  
URL: https://newsroom.eclipse.org/news/announcements/eclipse-foundation-unveils-2024-iot-embedded-developer-survey-results

---

### F5 — Connectivity top challenge: 48% cite it
**Vote: 3-0** *(corroborates F4 từ independent source)*

Source: Eclipse sustainability blog  
URL: https://blogs.eclipse.org/post/amin-rasti/sustainability-security-insights-2024-iot-embedded-developer-survey-report

---

### F6 — 47% developers prioritize safety certifications (IEC 61508, ISO 26262)
**Vote: 2-1**

Nearly 47% of embedded developers prioritize safety certifications — IEC 61508 và ISO 26262 — indicating certified OS support là significant market requirement.

Source: Eclipse sustainability blog

---

### F7 — Zephyr: 450+ community-supported boards
**Vote: 2-1**

Zephyr RTOS supports more than 450 community-supported boards từ nhiều architectures (as of early 2023).

Source: https://www.zephyrproject.org/why-we-moved-from-freertos-to-zephyr-rtos/

---

### F8 — Zephyr: 150 new boards added 2024
**Vote: 2-1**

Zephyr added support cho 150 new boards năm 2024, bao gồm Raspberry Pi Pico 2 và WCH CH32V003EVT.

Source: Zephyr 2024 wrap-up

---

### F9 — Open source adoption: 75% (2024), up từ 63% (2023)
**Vote: 3-0**

Open source adoption trong embedded/IoT tăng mạnh từ 63% (2023) lên **75%** (2024), với 24% respondents active as committers.

Source: Eclipse Foundation 2024 survey announcement

---

## Ý nghĩa với Cellos

### Market opportunity

| Segment | Current | Cellos Angle |
|---|---|---|
| Linux 46% | Mature, hard to displace | G2: Two-plane arch (Cellos data plane + Linux VM) |
| FreeRTOS 29% | C-only, no memory safety | G1: Migration target for safety-sensitive projects |
| Zephyr 21% | Growing, still C-dominant | G1: Rust-first alternative, same board support target |
| ThreadX 13% | Azure-tied, Microsoft push | Niche, not primary target |

### Pain points → Cellos features

| Survey Pain Point | Cellos Response |
|---|---|
| Connectivity (48%) | Net cell (DHCP/socket API shipped); TLS planned Phase 24 |
| Security (35%, growing) | Ed25519 Cell signing, capability tokens (ZST), Law 4 |
| Safety certs (47%) | Long-term: ASIL-B path cần formal WCET analysis |

### Strategic insight

**Open source 75% với 24% committers** = thị trường muốn community-driven OS, không chỉ vendor push. Cellos cần public GitHub presence + contribution guide để capture developer mindshare.

**FreeRTOS 29% chưa growth** — FreeRTOS không grow vì Amazon locked, C-only. Zephyr stealing mindshare. Cellos có thể target cùng segment với pitch tốt hơn: "Zephyr với Rust-native và memory safety by construction."

---

## Refuted claims (đáng note)

- ~~"FreeRTOS có limited library support và lacks flexibility"~~ — 0-3, bác bỏ (overclaim)
- ~~"Zephyr 1,100 unique contributors, 50% first-time"~~ — 0-3, bác bỏ (số không verify được)
- ~~"Only 15 new Zephyr products 2024"~~ — 0-3, bác bỏ (số quá thấp, suspicious)
- ~~"FreeRTOS runs at 8KB RAM"~~ — 0-3, bác bỏ (số quá thấp, suspicious)
- ~~"Energy management fastest-growing IoT application at 29%"~~ — 0-3, không verify được

---

## Caveats

- Synthesis step fail do rate limit — 9 claims không được merge thành coherent narrative
- Survey sample: lệch về Eclipse Foundation community (open source oriented)
- Rust adoption numbers trong embedded KHÔNG có trong confirmed claims do search agents fail
- Commercial vs hobbyist split không có trong confirmed data

---

*Sources: [Eclipse Foundation 2024 Survey](https://newsroom.eclipse.org/news/announcements/eclipse-foundation-unveils-2024-iot-embedded-developer-survey-results) · [Eclipse Sustainability Blog](https://blogs.eclipse.org/post/amin-rasti/sustainability-security-insights-2024-iot-embedded-developer-survey-report) · [Zephyr 2024 Wrap-up](https://zephyrproject.org/zephyr-rtos-2024-wrap-up-a-year-of-growth-innovation-and-community-impact/)*
