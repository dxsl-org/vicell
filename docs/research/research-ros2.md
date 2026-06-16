# Research: ROS 2 Architectural Pain Points

**Nguồn**: Deep research — 105 agents, 8 sources fetched, 33 claims → top 25 verified (7 confirmed / 18 killed)  
**Venues**: Casini et al. arXiv:2601.10722 (Dec 2025), Ishikawa-Aso & Kato IEEE ISORC 2025 (arXiv:2506.16882), Teper et al. IEEE (9355523), official ROS 2 docs, Autoware Foundation GitHub, ros2/rclcpp#272  
**Date**: 2026-06-08

---

## Tóm tắt executive

ROS 2 architectural pain points hội tụ về 3 root causes:

1. **Default executor's polling-point design** structurally prevents real-time callback prioritization — được acknowledge trong official docs, vẫn unfixed trong Jazzy (current LTS) và Kilted
2. **DDS internal threads** không controlled bởi OS scheduler, gây latency degradation under CPU load; config không portable cross DDS vendors
3. **Intra-process zero-copy ↔ fault isolation dichotomy**: ComponentContainer cần một trong hai, không thể có cả hai

Tất cả 3 root causes map **trực tiếp** sang ViCell advantages: Cellular SAS + LBI giải quyết đồng thời cả 3 mà không có architectural trade-off.

**ViCell G1 robot target**: ARM64/RV64 SBC, competing với ROS 2 + Linux deployment.

---

## Findings đã xác nhận (adversarially verified)

### F1 — Default executor: priority inversion by design (structural)
**Confidence: HIGH** | Vote: 3-0

ROS 2 default executor có structural priority-inversion defect: tại mỗi processing window, at most one callback instance per entity được admit vào wait set. Higher-priority callbacks trở nên ready trong processing window bị excluded và lower-priority callbacks đã có trong ready set được execute trước.

**Hệ quả**: Official ROS 2 docs acknowledge: *"Callbacks may suffer from priority inversion. Higher priority callbacks may be blocked by lower priority callbacks."* Paper kết luận: "difficult, or even impossible, to properly prioritize callbacks."

**Status**: Vẫn unfixed trong Jazzy (May 2024 LTS) và Kilted (May 2025). Kilted thêm experimental events executor cho **rclpy only** — C++ rclcpp default executor không thay đổi.

**Sources**:
- Casini et al. arXiv:2601.10722v1 (Dec 2025) — formal timing diagram
- Teper et al. IEEE 9355523 — "previously recognized"
- Casini et al. arXiv:2512.16926 "Bridging the Gap" (Dec 2025)
- Official ROS 2 Rolling docs

**ViCell mapping**: RT Cell với pinned core + preemptive scheduler loại bỏ executor-level scheduling hoàn toàn. Mỗi Cell là independent task, kernel scheduler quyết định priority, không có polling-point inversion.

---

### F2 — DDS internal threads: uncontrolled scheduling + non-portable config
**Confidence: HIGH** | Vote: 3-0 (load degradation); 2-1 (non-portability)

DDS internal threads (flow-controller, receive threads) không được pin vào real-time scheduling priorities và compete for CPU time under load. CycloneDDS exhibit increasing coefficient of variation (CV) trong message latency khi CPU utilization tăng — paper attribute directly đến "scheduling delays of various involved threads, including DDS threads."

Config không portable: CycloneDDS dùng CYCLONEDDS_URI XML, RTI Connext dùng QoS profiles, FastDDS dùng FASTDDS_DEFAULT_PROFILES_FILE — không có portable rmw abstraction cho flow-controller thread scheduling policy.

**Sources**:
- Ishikawa-Aso & Kato, IEEE ISORC 2025 (arXiv:2506.16882v1) — benchmark CycloneDDS trên Intel Xeon E-2278GE với Linux 6.2, SCHED_FIFO subscriber, 100KB messages, 0/30/60/90% CPU load
- Casini et al. arXiv:2601.10722v1
- Official ROS 2 DDS tuning docs (docs.ros.org/en/rolling/How-To-Guides/DDS-tuning.html)

**ViCell mapping**: RT Cell với pinned core + SCHED_FIFO loại bỏ vấn đề này hoàn toàn — IPC path **không bao giờ** involve uncontrolled third-party thread. Typed vtable IPC là direct Rust fn pointer call, không có middleware thread.

**Caveat**: Paper (2506.16882v1) main contribution là Agnocast — CycloneDDS là baseline bị beat. Test conditions (100KB, SCHED_OTHER DDS threads) có thể được chọn để maximize contrast.

---

### F3 — ComponentContainer: zero-copy XOR fault isolation
**Confidence: HIGH** | Vote: 3-0

ROS 2 intra-process zero-copy qua ComponentContainer compromises fault isolation: nodes cùng OS process để achieve true zero-copy → single node crash (SIGSEGV, abort, unhandled exception) kills all co-located nodes.

**Autoware Foundation**: Explicitly lists phasing out ComponentContainer là long-term goal vì trade-off này. `component_container_isolated` variant dùng per-component executors nhưng vẫn là one OS process → SIGSEGV trong bất kỳ component nào vẫn kill container.

**Sources**:
- IEEE ISORC 2025 (arXiv:2506.16882v1): "true zero-copy communication can be achieved... However, this method compromises fault isolation... the failure of a single node's process can lead to complete system failure."
- Official ROS 2 composition docs (docs.ros.org/en/rolling/Concepts/Intermediate/About-Composition.html)
- Autoware Foundation GitHub discussion #5835

**ViCell mapping**: **Đây là strongest competitive advantage của ViCell.** Cellular SAS + LBI achieve true zero-copy (same address space, typed vtable IPC, no serialization) **WHILE** maintaining fault isolation via Rust type system và never-die supervisor. Dichotomy này không tồn tại trong ViCell.

---

### F4 — Polymorphic allocators lock-in: zero-copy cho dynamic types architecturally infeasible
**Confidence: HIGH** | Vote: 3-0

Zero-copy cho dynamic/unsized message types (LiDAR point clouds, image buffers) trong ROS 2 yêu cầu polymorphic allocators (`std::pmr`). Nhưng:
- `std::pmr` container types không compatible với existing code (type-incompatible)
- Official zero-copy design chỉ support loaned messages cho POD types
- ros2/rosidl#566 requesting polymorphic allocator adoption open nhiều năm, không có assignee

Paper kết luận: *"Converting all container types... to use polymorphic allocators could theoretically solve this incompatibility, but such extensive modifications across the entire ecosystem are impractical."*

Agnocast project (TIER IV, production Autoware) workaround bằng separate IPC mechanism vì allocator path không viable.

**Sources**:
- IEEE ISORC 2025 (arXiv:2506.16882v1)
- Official ROS 2 zero-copy design doc (design.ros2.org/articles/zero_copy.html)
- GitHub ros2/rosidl#566 (open, no assignee)

**ViCell mapping**: ViCell typed vtable IPC với grant-based shared memory (GrantAlloc/Share/Slice, syscalls 208-212) cung cấp zero-copy cho arbitrary message sizes bao gồm dynamic types, không có allocator compatibility constraint.

---

### F5 — C++ memory safety: use-after-free tại executor boundary (historical, nhưng illustrative)
**Confidence: MEDIUM** | Vote: 3-0 (documented behavior; bug đã fix)

Node added via temporary `shared_ptr` (destroyed immediately after `add_node`) gây executor crash với SIGABRT qua `std::system_error` thrown inside DDS/RMW guard condition mutex lock path. Root cause: executor wait set retain stale guard condition pointer sau khi node's `shared_ptr` expire, causing use-after-free trên next `rcl_wait` call.

Filed bởi Morgan Quigley (ROS co-creator) ở ros2/rclcpp#272. Fixed trong PR #741 (~2017-2018).

**Relevance**: Bug cũ nhưng illustrate structural class: C++ shared ownership tại executor/middleware boundary là fragile. Pattern risk tồn tại dù specific bug đã fix.

**ViCell mapping**: Law 4 (`#![forbid(unsafe_code)]` trong Cells) và Rust ownership semantics làm class use-after-free này impossible trong Cell code. Never-die supervisor provide containment layer cho kernel/HAL unsafe code.

---

### F6 — No architectural crash isolation: node crash propagates to executor
**Confidence: MEDIUM** | Vote: derived từ 3-0 claims

ROS 2 không có architectural mechanism để isolate one-node crash khỏi executor: SIGABRT hoặc unhandled exception trong bất kỳ node nào trong shared executor/process terminate toàn bộ executor. Không có standard supervisor hoặc watchdog primitive trong core framework để restart failed nodes.

Lifecycle node API cung cấp state management nhưng **không** cung cấp crash recovery — lifecycle node crash trong ACTIVE state vẫn kill container. Developers phải dùng custom launch_ros.actions.LifecycleNode monitors hoặc external watchdogs.

**Sources**: IEEE ISORC 2025, official ROS 2 composition docs, Autoware #5835

**ViCell mapping**: Never-die supervisor (NotifyOnExit=204, init auto-restarts vfs/net/shell) directly address gap này. Cell fault isolation via Rust LBI: Cell panic bị catch tại Cell boundary và không propagate đến scheduler hoặc other Cells.

---

### F7 — DDS flow-controller scheduling: non-portable config (standalone confirmation)
**Vote: 2-1**

Claim này corroborates F2 về non-portability từ independent angle.

---

## Benchmark missing: ViCell vs ROS 2

**Câu hỏi quan trọng nhất chưa có câu trả lời**:

> *Latency distribution của ViCell typed vtable IPC (GrantAlloc path) vs CycloneDDS intra-host trên ARM64 SBC (Jetson, RPi 5) dưới cùng CPU load conditions như IEEE ISORC 2025?*

Đây là benchmark validation trực tiếp nhất cho G1 robot positioning. Cần prioritize sau khi network + input subsystems ổn định.

---

## Open Questions

1. Experimental events executor (Kilted, rclpy only) có resolve polling-point priority inversion cho Python nodes không, và có timeline cho C++ rclcpp equivalent?
2. Agnocast (TIER IV) có viable migration path cho existing ROS 2 fleets không, hay bị cùng ecosystem adoption barrier như polymorphic allocators?
3. ViCell never-die supervisor để fully replace ROS 2 lifecycle node pattern cần Cell-level state machine gì (INACTIVE/ACTIVE/FINALIZED equivalent) — specs/12-reliability.md có specify không?

---

## Caveats

- Quantitative latency claims (50% overhead, 80x WCET tighter bounds, O(N²) DDS scaling) đều bị refute hoặc không verify được do session limit. Directional findings (latency degrades, DDS non-portable) confirmed; magnitudes không.
- ROSCon 2021-2024 talks và r/ROS community threads không có trong surviving claims — synthesis weighted toward academic literature.
- Finding [4] (SIGABRT bug) là historical (2017-2018 fix) — không cite như current reproducible bug.

---

*Sources: [Casini et al. 2025](https://arxiv.org/html/2601.10722v1) · [Ishikawa-Aso & Kato IEEE ISORC 2025](https://arxiv.org/html/2506.16882v1) · [Teper et al. IEEE](https://ieeexplore.ieee.org/document/9355523/) · [ROS 2 Composition Docs](https://docs.ros.org/en/rolling/Concepts/Intermediate/About-Composition.html) · [Autoware #5835](https://github.com/autowarefoundation/autoware/discussions/5835) · [ros2/rosidl#566](https://github.com/ros2/rosidl/issues/566)*
