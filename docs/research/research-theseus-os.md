# Research: Theseus OS — LBI Academic Kin, Competitive Analysis

**Version**: 1.0
**Last Updated**: 2026-06-11
**Phương pháp**: 34 tool calls, 11+ sources fetched, 7 findings adversarially verified
**Report đầy đủ**: `.agents/reports/research-260611-theseus-os.md`

> **Kết luận một câu**: Theseus proves the SAS+LBI model; Cellos operationalizes it.
> Dùng Theseus làm academic credibility (OSDI 2020), không phải threat.

---

## Tóm tắt executive

| Dimension | Theseus | Cellos | Cellos ahead? |
|---|---|---|---|
| Architecture | SAS + intralingual (Rust only) | SAS + LBI + Rust | Same philosophy |
| IPC cost (measured) | 687 cycles fastpath | ~2–3 cycles (vtable, planned) | Cellos design target far cheaper |
| Hot-swap | 385µs median, published eval | Zombie state + disk re-link, no numbers | Theseus has published eval, Cellos does not |
| RISC-V | ❌ Not supported | ✅ Primary target | **Cellos +++ decisive** |
| Persistent FS | ❌ Memory-only | ✅ FAT32 + littlefs | **Cellos +++** |
| Async executor | ❌ Unmerged branch (busy-polling) | ✅ Kernel async IPC | **Cellos +++** |
| Real-time | ❌ No RT design | ✅ RT watchdog + priorities | **Cellos +++** |
| Production-oriented | ❌ Research only | ✅ Never-die, supervisor, OTA | **Cellos +++** |
| Formal verification | ✅ Hybrid type+proof (2024 paper) | ❌ Not pursued | Theseus + (academic only) |
| C/legacy code | ⚠️ breaks intralingual guarantee | ✅ Tier 1b FFI (RKNN/K230) | **Cellos +++** |

---

## Findings đã xác nhận

### F1 — "Intralingual design" = SAS+LBI với compiler làm sole enforcer
**Confidence: HIGH** | OSDI 2020 + Theseus Book

Theseus chạy tất cả (kernel, drivers, services, apps) trong **single address space + single privilege level**. Không có ring-0/ring-3 boundary. Mọi cell là một Rust crate được compile thành relocatable `.o` và loaded vào cùng address space tại runtime.

"Intralingual" nghĩa là OS khớp execution environment với Rust's runtime model: affine ownership, borrow checker, no GC, compiler-enforced lifetimes. **Không có garbage collector.** Memory management hoàn toàn là Rust ownership — `MappedPages` type represents owned memory region; drop = return frames.

Isolation là **purely software-defined** — không có hardware enforcement của cell boundaries. Compiler và type system là cơ chế duy nhất.

**Cellos comparison**: Gần như identical philosophy. Cellos thêm per-task page tables (User VA < 0x8000_0000) như belt-and-suspenders layer trên LBI — Theseus deliberately omits hardware guard layer.

**Sources**: [OSDI 2020](https://www.academia.edu/51083894/Theseus_an_Experiment_in_Operating_System_Structure_and_State_Management) · [Theseus Book](https://www.theseus-os.com/Theseus/book/design/design.html)

---

### F2 — "State spill" là core problem Theseus giải quyết — Cellos's Law 2 là cùng giải pháp
**Confidence: HIGH** | PLOS 2017 + OSDI 2020

State spill: callee retain state về caller vượt quá duration của interaction → caller's future correctness phụ thuộc vào callee's memory of prior state. Theseus eliminate bằng cách require servers **stateless with respect to clients** — tất cả state phải được pass in request itself (client owns it).

**Đây là chính xác Cellos's Law 2**: `async fn process(data: Box<[u8]>) -> Box<[u8]>` thay vì `async fn process(data: &mut [u8])`. Theseus verify điều này tại architectural level qua per-section dependency metadata; Cellos enforce qua type system.

State spill freedom enables live upgrade: không có component nào hold state của component khác → swap component chỉ cần relinking, không cần state transfer.

**Sources**: [PLOS 2017](https://dl.acm.org/doi/pdf/10.1145/3144555.3144560) · [OSDI 2020](https://www.academia.edu/51083894/Theseus_an_Experiment_in_Operating_System_Structure_and_State_Management)

---

### F3 — Theseus IPC: 687 cycles fastpath, 1,664 cycles async channel
**Confidence: HIGH** | 3 independent sources (OSDI 2020 eval, academia.edu, course notes)

Theseus IPC model: **ITC channels** — typed shared-memory channels backed by atomic references trong SAS. Không có kernel involvement sau khi established.

| IPC type | Theseus | Cellos |
|---|---|---|
| Channel async RTT | 1,664 cycles | ~100–1,000 cycles (kernel-mediated, current) |
| ITC fastpath RTT | **687 cycles** | ~2–3 cycles (vtable, planned Phase 27) |
| seL4 comparison | ~802 cycles RTT (equivalent) | — |

Theseus fastpath tương đương seL4 microkernel về RTT — SAS eliminates context switch, partially compensating cho loss of hardware optimization. Cellos's vtable design target (~2–3 cycles) vẫn là 230× cheaper hơn Theseus fastpath.

**Sources**: [academia.edu OSDI 2020 ITC measurements](https://www.academia.edu/51083894/Theseus_an_Experiment_in_Operating_System_Structure_and_State_Management)

---

### F4 — Hot-swap: 385µs median, 69% hardware fault recovery
**Confidence: HIGH** | OSDI 2020 published evaluation

Theseus hot-swap là 4-stage atomic process:
1. Load new cell trong isolation
2. Verify bidirectional dependency graph
3. Rewrite all relocation entries từ old → new cell sections
4. Atomically swap symbol map entry; drop old cell khi ref-count → 0

**Published numbers:**
- Evolving ITC channel module: 19.5ms
- Evolving scheduler: 21.3ms
- Evolving e1000 network driver: 65.6ms
- **Median hot-swap downtime: 385µs**
- Hardware fault recovery: **69% của 664 manifest faults** từ 800,000 injected faults

31% failure rate = async unwinding gaps (LLVM chỉ emit unwind tables cho Rust exception points, không phải arbitrary hardware fault points).

**Cellos implication**: Cellos có Zombie State + ref-count drain + disk re-link mechanism nhưng **chưa có published benchmark numbers**. Đây là credibility gap cần close.

**Sources**: [academia.edu OSDI 2020 evaluation section](https://www.academia.edu/51083894/Theseus_an_Experiment_in_Operating_System_Structure_and_State_Management)

---

### F5 — Tại sao Theseus không ship production: 4 hard structural blockers
**Confidence: HIGH** | Paper acknowledgements + GitHub inspection

1. **No POSIX**: Paper explicitly: "the system lacks full POSIX support and standard library." Shell cần carve-outs từ intralingual rules.
2. **Pure-Rust mandate**: Bất kỳ C/C++ library nào (curl, OpenSSL, SQLite) đều breaks compiler knowledge chain. Đây là architectural fundamental, không phải gap tạm thời.
3. **No async in mainline**: `Theseus_async` branch (busy-polling) **never merged**. No `async/await` runtime = no modern Rust network stack.
4. **x86-64 only** (production quality): aarch64 = "most core subsystems complete" nhưng "full builds not supported". **Zero RISC-V.**

Ngoài ra: memfs + heapfile (RAM only, không có block-backed persistent FS), không có DHCP, không có RT guarantees.

**Cellos comparison**: Cellos giải quyết tất cả 4 blockers: RISC-V primary, Tier 1b C FFI, async kernel, FAT32+littlefs persistent FS.

**Sources**: [academia.edu OSDI 2020 acknowledged limitations](https://www.academia.edu/51083894/Theseus_an_Experiment_in_Operating_System_Structure_and_State_Management) · [GitHub kernel directory inspection](https://github.com/theseus-os/Theseus/tree/theseus_main/kernel)

---

### F6 — Paper landscape 2017–2025: 6 papers, narrowing to formal verification
**Confidence: HIGH** | Theseus papers page

| Year | Venue | Focus |
|---|---|---|
| 2017 | EuroSys | Problem definition (state spill characterization) |
| 2017 | PLOS | Early prototype |
| 2020 | OSDI | **Core paper** — full design + eval |
| 2022 | OSDI poster | Driver verification |
| 2023 | KISV | Type-system-aided formal proof |
| 2024 | arxiv 2501.00248 | Hybrid type+proof: "slightly lessens guarantee strength" |

Từ 2023 trở đi: research agenda thu hẹp về formal correctness proofs, **không phải operational completeness**. Không có paper nào về production deployment, real hardware, hay networking completeness.

**Sources**: [Theseus papers page](https://www.theseus-os.com/Theseus/book/misc/papers_presentations.html) · [arxiv 2501.00248](https://arxiv.org/abs/2501.00248)

---

### F7 — Maintenance status: active research, không phải abandonment
**Confidence: HIGH** | GitHub activity + arxiv submission date

3,200 GitHub stars, automated CI passing (build + clippy + QEMU), Discord community, Yale YECL lab involvement. arxiv paper submitted Dec 31, 2024 confirms active research. **Không phải archived.**

Nhưng: no company backing, no roadmap to production, bus factor thấp (Kevin Boos PhD completed, small team), no crates.io presence.

**Sources**: [GitHub Theseus](https://github.com/theseus-os/Theseus) · [arxiv 2501.00248](https://arxiv.org/abs/2501.00248)

---

## Cellos vs Theseus — Gap Matrix đầy đủ

| Capability | Cellos | Theseus |
|---|---|---|
| **Persistent FS** | FAT32 + littlefs, VFS MountTable | `memfs` + `heapfile` RAM only |
| **Network stack** | smoltcp DHCP/TCP/UDP, socket API | Basic ping + e1000 driver, http_client exists (maturity unknown) |
| **RISC-V** | ✅ Primary target (RV64GC full) | ❌ Not supported, not planned |
| **ARM64** | HAL stubs complete, ring-3 smoke | "Most core subsystems" but no full build |
| **Async executor** | ✅ Kernel async IPC + RecvTimeout | ❌ Unmerged busy-polling branch |
| **Real-time** | RT watchdog, 3-tier priorities | ❌ No RT design or guarantees |
| **Fault recovery** | init supervisor, auto-restart, catch_unwind | 69% hardware fault recovery (formal eval) |
| **Hot-swap** | Zombie state + disk re-link | 385µs median, 4-stage atomic (published eval) |
| **Shell** | Full shell, pipes, redirect, Lua | Basic terminal |
| **Never-die** | P00–P05 reliability suite | Academic fault injection experiments |
| **C/legacy code** | Tier 1b FFI (nncase/RKNN), tlibc | `tlibc` breaks intralingual guarantee |
| **Formal verification** | Not pursued | Hybrid type+proof (2024 paper) |

---

## Positioning cho Cellos

**① Dùng Theseus làm academic validation, không phải threat**
OSDI 2020 là peer-reviewed A\* venue. Khi Cellos bị skepticism về "tại sao không hardware isolation?", cite Theseus + Singularity để validate SAS+LBI architecture với zero cost.

**② Positioning narrative**: "Theseus proves the model; Cellos operationalizes it."
- Theseus: same architectural bet, optimized for research purity
- Cellos: same architectural bet, optimized for production deployment

**③ Nên close credibility gap**: Instrument hot-swap latency và publish numbers tương đương Theseus's 385µs benchmark. Cellos có mechanism, thiếu published eval.

---

## Caveats

- OSDI 2020 PDF không parseable; numbers từ academia.edu HTML render + course notes (3 independent sources agree)
- Theseus network stack completeness không verify được đầy đủ (http_client crate exists nhưng maturity unknown)
- Hot-swap correctness comparison với Cellos's Zombie State mechanism không verified qua source code

---

*Sources: [Theseus GitHub](https://github.com/theseus-os/Theseus) · [OSDI 2020](https://www.academia.edu/51083894/Theseus_an_Experiment_in_Operating_System_Structure_and_State_Management) · [PLOS 2017](https://dl.acm.org/doi/pdf/10.1145/3144555.3144560) · [Theseus Book](https://www.theseus-os.com/Theseus/book/design/design.html) · [arxiv 2501.00248](https://arxiv.org/abs/2501.00248) · [KISV 2023](https://doi.org/10.1145/3625275.3625398)*
