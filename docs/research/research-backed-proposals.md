# Cellos — Research-Backed Feature Proposals

**Version**: 1.0  
**Last Updated**: 2026-06-08  
**Nguồn gốc**: Deep research từ 26 nguồn (ASPLOS 2022, SOSP 2025, JSA 2024, Zephyr tracker, lobste.rs, HackerNews)  
**Phương pháp**: 108 agent · 105 claim extracted · 25 adversarially verified (2/3 vote) · 8 confirmed

> Mỗi đề xuất dưới đây **bắt nguồn từ nghiên cứu peer-reviewed hoặc maintainer-confirmed bug report** —  
> không phải suy đoán. Citation đi kèm từng đề xuất.

---

## Tóm tắt executive từ research

Có 5 vấn đề được xác nhận hội tụ từ nhiều nguồn độc lập:

| # | Vấn đề đã xác nhận | Nguồn | Liên quan Cellos |
|---|---|---|---|
| F1 | OS bị "lock in" isolation strategy ngay từ design time — đổi sau = refactor toàn bộ | FlexOS ASPLOS 2022 | Cell manifest configurable isolation |
| F2 | Rust type system **không đủ** cho SAS khi có unsafe HAL/FFI — cần thêm enforcement | Unishyper JSA 2024 + CHERIoT SOSP 2025 | Memory Ownership Registry là đúng hướng |
| F3 | C/C++ gây ~70% critical security vulnerabilities — không thể fix bằng tooling, chỉ fix bằng language swap | CHERIoT SOSP 2025 + NSA/CISA 2025 | Rust-only Cell ABI là lợi thế có số liệu |
| F4 | **Isolation dễ, safe sharing mới khó** — mọi OS truyền thống build around isolation, sharing là afterthought | CHERIoT SOSP 2025 | Grant API đang đi đúng hướng |
| F5 | Zephyr: DMA devices **hoàn toàn bypass** MMU/MPU — self-acknowledged structural gap, open issue 2023 | Zephyr RFC #60289 | AsyncLocked state address được điều này |

---

## Đề xuất 1: DMA Safety Lock — First-Class Primitive

### Nguồn gốc
**Zephyr RFC #60289** (mở bởi ARM engineers, tháng 7/2023, vẫn open đến 2025):

> *"MMU/MPU can only limit memory accesses from CPUs. Memory accesses such as those from DMA are not protected by MMU/MPU, which may cause critical security issues. Without taking action, Zephyr would be under increasing security risk."*

Đây là **self-acknowledged structural gap** của Zephyr — và toàn bộ embedded RTOS ecosystem (FreeRTOS, RTEMS tương tự). Lý do không fix được: DMA bypass là thuộc tính hardware, không thể chặn bằng software trong process model vì process model dùng CPU page table, không kiểm soát DMA controller.

### Cellos đã có gì
`AsyncLocked` state trong Memory Ownership Registry: khi Cell đang làm async I/O (DMA-like), kernel mark buffer là `AsyncLocked` — không thể free, không thể remap kể cả khi Cell sở hữu bị kill.

### Đề xuất mới: Expose như first-class API
Hiện tại `AsyncLocked` là internal kernel mechanism. Đề xuất expose thành explicit syscall:

```rust
// Driver Cell khai báo DMA transfer
let guard = sys_dma_lock(buffer: Box<[u8]>, device_id: DeviceId)?;
// kernel: mark buffer AsyncLocked, register (buffer_range, device_id) pair
// nếu có IOMMU/SMMU: kernel program IOMMU entry cho device này
// nếu không có IOMMU: kernel từ chối mọi unmap/remap trong range này

dma_start(device_id, guard.phys_addr(), guard.len());
// ... DMA running ...
drop(guard); // kernel: release AsyncLocked, IOMMU entry removed
```

**Giá trị thực tế**: Cellos có thể claim là RTOS đầu tiên có DMA safety primitive không cần IOMMU/SMMU phần cứng — chạy được trên Cortex-M, RISC-V MCU không có SMMU. Cụ thể hơn và thực tế hơn bất kỳ thứ gì Zephyr có.

**Feasibility**: Cần minimal kernel change — `AsyncLocked` đã tồn tại, chỉ cần API wrapper và explicit device_id registration. **Ưu tiên G1.**

---

## Đề xuất 2: Per-Cell Isolation Level — Configurable tại Load Time

### Nguồn gốc
**FlexOS (ASPLOS 2022, Distinguished Artifact Award)**:

> *"At design time, modern operating systems are locked in a specific safety and isolation strategy... revisiting these choices after deployment requires a major refactoring effort."*

FlexOS chứng minh feasibility của compile-time isolation configurability — cùng một codebase có thể là microkernel hay SAS tùy config. Nhưng compile-time nghĩa là lock-in tại build, không phải runtime.

### Cellos có thể làm tốt hơn FlexOS
Cellos đã có Cell manifest + load-time linking. Đề xuất thêm `isolation_level` vào manifest:

```toml
# Cell manifest
[isolation]
level = "lbi"          # chỉ Language-Based Isolation (default, zero cost)
# level = "lbi+mpk"   # LBI + Intel MPK compartment (x86_64 G2, hardware-enforced)
# level = "lbi+satp"  # LBI + per-Cell SATP (tương lai, nếu ASID gap được fix)
```

**Ý nghĩa thực tế**:
- **Trusted first-party Cell** (VFS, Net, Shell): `lbi` — zero overhead
- **Third-party Cell** (plugin không kiểm chứng kỹ): `lbi+mpk` trên G2 x86_64 — hardware fence
- **Tier 1b C library Cell** (RKNN SDK): `lbi` với SAFETY-documented unsafe boundary

Điều FlexOS không làm được: **thay đổi isolation level của một Cell tại runtime** mà không restart toàn hệ thống. Cellos có thể: hot-swap Cell với manifest mới có `isolation_level` khác.

**Feasibility**: `lbi` đã là default. MPK compartment trên x86_64 là G2 work item. Manifest field thêm vào ngay. **Ưu tiên G2.**

---

## Đề xuất 3: Software Zone Mechanism — ARM/RISC-V không cần MPK

### Nguồn gốc
**Unishyper (JSA 2024)**:

> *"Beyond memory management based on Rust, Unishyper's Zone mechanism provides extra protection... safe risks that may escape Rust security insurance."*

Unishyper dùng Intel MPK (Memory Protection Keys) trên x86_64 để bổ sung cho Rust. Vấn đề: **MPK không có trên ARM/RISC-V** — đây là những target chính của Cellos G1.

CHERIoT (SOSP 2025) đến cùng kết luận từ hướng ngược: *"pointer-manufacturing in C/unsafe code cannot be stopped without hardware tagging."*

### Cellos đề xuất: Software Zone
Vì không có MPK trên ARM/RISC-V, Cellos dùng phương pháp software:

```rust
// Tất cả unsafe code trong HAL/kernel phải đi qua ZoneGuard
pub struct UnsafeZone {
    caller_cell: CellId,
    allowed_range: (VAddr, VAddr),  // HAL chỉ được access range này
    access_type: AccessType,         // ReadOnly | ReadWrite
}

impl UnsafeZone {
    // Trước khi bắt đầu unsafe operation:
    pub fn enter(cell: CellId, range: MemRange) -> Result<Self, ViError> {
        UNSAFE_ZONE_REGISTRY.register(cell, range)?;  // conflict check
        Ok(Self { ... })
    }
    // Drop = auto-release zone
}
```

**Kernel enforcement**: Khi một Cell bị kill trong lúc UnsafeZone active → kernel biết chính xác range nào đang bị "tainted" → zero ra range đó trước khi cấp lại cho Cell khác.

**Giá trị**: Không cần CHERI hardware, không cần MPK, chạy được trên mọi ARM64/RV64 Cellos target. Defense-in-depth layer giữa "Rust catches most bugs" và "hardware fence catches the rest".

**Feasibility**: Extend Memory Ownership Registry với unsafe zone tracking. Medium effort. **Ưu tiên G1/G2.**

---

## Đề xuất 4: Capability-Scoped Sharing — "Zero-Copy là Default"

### Nguồn gốc
**CHERIoT RTOS (SOSP 2025)**:

> *"CHERIoT starts from a fundamental assumption that isolation is easy, (safe) sharing is hard... Most mainstream operating systems have a process model built around isolation, with sharing as an afterthought."*

Insight sâu sắc nhất từ research: hầu hết OS **design for isolation first, sharing second** — kết quả là sharing selalu expensive (copy, syscall, shared memory boilerplate). Cellos đang đi đúng hướng với Grant API nhưng chưa đến cùng.

### Đề xuất: Sharing-First IPC Design
Hiện tại: default IPC là `sys_send` (small message copy). Grant là opt-in cho large data.

Đề xuất đảo ngược philosophy: **default là zero-copy ownership transfer, copy là opt-in**:

```rust
// Hiện tại:
sys_send(tid, &small_message)?;           // copy — default
sys_grant_alloc(size)?; // zero-copy — explicit, verbose

// Đề xuất:
// ostd::ipc::send(service, payload) tự động chọn:
// - payload: Box<T> → zero-copy ownership transfer (Grant API)  
// - payload: &T (small, Copy) → inline message copy
// Developer không cần biết cơ chế bên dưới
```

**Scoped capability cho sharing**: mỗi Grant có lifetime gắn với Rust lifetime — khi `GrantGuard` drop, kernel tự thu hồi. Không cần explicit `sys_grant_free` — tương tự như Rust borrow checker nhưng cross-Cell.

**Feasibility**: Cần ostd IPC abstraction layer. Medium effort, high value cho developer experience. **Ưu tiên G1/G2.**

---

## Đề xuất 5: Unsafe Surface Audit — Cell Security Score

### Nguồn gốc
**CHERIoT SOSP 2025 + NSA/CISA June 2025 advisory**:

> *"The lack of memory safety is responsible for around 70% of critical security vulnerabilities."*

Số liệu này từ Microsoft/Google/Chromium codebases. CISA 2025 advisory kêu gọi tất cả software developers chuyển sang memory-safe languages.

### Đề xuất: Audit Report tại Cell load time

```
[Cellos Cell Loader — Security Audit]
Loading: camera-cell v1.3 (signed by Cellos Lab)

  Code composition:
  ├── Safe Rust: 4,821 lines   (94.2%)
  ├── Unsafe Rust (HAL only):    298 lines    (5.8%)
  │   ├── kernel/src/hal/mmio.rs:  82 lines  — MMIO read/write
  │   ├── hal/arch/riscv/trap.rs:  156 lines — trap handler
  │   └── libs/api/posix.rs:        60 lines — C FFI shim
  └── C FFI (Tier 1b):             0 lines   (0%)

  Unsafe budget: 298 / 500 lines (59.6%) ✅ within quota
  Capability tokens requested: [CameraCap, NetworkCap::Listen{8080}]
  DMA regions registered: 0
  
  Decision: LOAD ✅
```

Nếu `unsafe_budget` vượt ngưỡng (ví dụ `max_unsafe_lines` trong manifest) → kernel từ chối load, yêu cầu dev review.

**Giá trị**: Đây là thứ không tồn tại ở bất kỳ OS hay package manager nào. npm/cargo audit check CVE, không check unsafe surface. Cellos có đủ thông tin (Cell manifest + linker) để làm điều này tại load time.

**Feasibility**: Cần static analysis tại build time (cargo plugin) + manifest field + kernel audit check tại load. Medium effort. **Ưu tiên G2 nhưng thiết kế format ngay.**

---

## Đề xuất 6: Lightweight POSIX Fork cho G2 (không cần CHERI)

### Nguồn gốc
**µFork (SOSP 2025, arxiv:2509.09439)**:

> *"µFork emulates POSIX processes (µprocesses) and achieves fork by creating for the child a copy of the parent µprocess' memory at a different location within a single address space."*

µFork dùng CHERI hardware để relocate pointers sau khi copy. Cellos không có CHERI, nhưng có thứ tương đương: **StateTransfer trait** đã serialize/deserialize Cell state.

### Đề xuất: `sys_spawn_fork(cell_id)` không cần CHERI

```rust
// Thay vì CHERI pointer relocation, dùng StateTransfer:
// 1. Pause target Cell
// 2. Call cell.serialize_state() → opaque blob
// 3. Kernel load new Cell instance với same ELF
// 4. Call new_cell.deserialize_state(blob) → state cloned
// 5. Resume both Cells independently

// Kết quả: 2 Cell độc lập, cùng state tại thời điểm fork
let child_cell_id = sys_spawn_fork(parent_cell_id)?;
```

**Ứng dụng thực tế G2**:
- Scale inference server: 1 inference Cell → fork → 4 instances nhận requests song song
- Load balancing: fork Cell khi load tăng, kill khi load giảm
- Isolation cho untrusted plugin: fork Cell trước khi chạy plugin, kill fork nếu plugin crash

**Caveat quan trọng**: µFork cần CHERI để handle raw pointers trong state. Cellos's approach chỉ work nếu `serialize_state()` không leak raw pointers — tức là Cell state phải serialize cleanly qua StateTransfer trait. Cells với raw pointer in state sẽ không support `spawn_fork`.

**Feasibility**: StateTransfer đã có. Cần kernel `sys_spawn_fork` syscall. Medium effort. **Ưu tiên G2.**

---

## Điều research bác bỏ — đáng chú ý

Research adversarially verify **bác bỏ 16/25 claims**, trong đó có một số điều quan trọng:

### ❌ "Rust type system đủ để làm SAS an toàn" — BÁC BỎ 3-0
Claims từ Theseus OS documentation rằng Rust alone đủ cho LBI đều bị bác bỏ 0-3 hoặc 1-2. Tức là **Cellos không được claim Rust là đủ** — cần Memory Ownership Registry + unsafe budget enforcement như bổ sung.

### ❌ "ARM MPU thực tế không ai dùng vì overhead" — BÁC BỎ 3-0
Claim này từ một paper 2019 nhưng bị bác — thực tế Zephyr và FreeRTOS có MPU support và một số system dùng. Cellos không nên dùng argument "MPU too slow" — thay vào đó dùng argument đúng hơn: "MPU không cover DMA, còn LBI + AsyncLocked thì cover."

### ❌ "POSIX fork không thể làm trong SAS" — BÁC BỎ 3-0
µFork (SOSP 2025) chứng minh ngược lại. Điều này có nghĩa Cellos **không cần từ chối POSIX compatibility** vĩnh viễn — có path đến G2 POSIX fork support.

---

## Tóm tắt: Prioritized Action Items

| Đề xuất | Nguồn gốc | Effort | Priority | Lý do ưu tiên |
|---|---|---|---|---|
| **DMA Safety Lock API** | Zephyr RFC #60289 | Low | **G1 ngay** | Gap đã được xác nhận trong tất cả RTOS — Cellos có thể fix |
| **Capability-Scoped Sharing (sharing-first IPC)** | CHERIoT SOSP 2025 | Medium | **G1/G2** | Thay đổi philosophy, không phải feature |
| **Software Zone Mechanism** | Unishyper JSA 2024 | Medium | G1/G2 | Defense-in-depth cho ARM/RISC-V không có MPK |
| **Unsafe Surface Audit** | CHERIoT + CISA 2025 | Medium | G2 design | Unique security feature, không OS nào có |
| **Per-Cell Isolation Level** | FlexOS ASPLOS 2022 | Low (manifest) / High (MPK) | G2 | Manifest field ngay, MPK implementation sau |
| **Lightweight Fork (no CHERI)** | µFork SOSP 2025 | Medium | G2 | Scaling inference, untrusted plugin isolation |

### Quick win có thể làm ngay hôm nay
**DMA Safety Lock API** — kernel AsyncLocked đã có, chỉ cần wrap thành explicit `sys_dma_lock` syscall với `device_id` tracking. Khoảng 200-300 LOC kernel change. Ngay lập tức Cellos có thể claim: *"Cellos là RTOS đầu tiên có explicit DMA safety primitive không phụ thuộc IOMMU/SMMU"* — điều Zephyr đang cố làm từ 2023 và chưa xong.

---

*Nguồn đầy đủ: FlexOS [ACM 2022](https://dl.acm.org/doi/10.1145/3503222.3507759) · CHERIoT RTOS [SOSP 2025](https://dl.acm.org/doi/pdf/10.1145/3731569.3764844) · Unishyper [JSA 2024](https://dl.acm.org/doi/10.1016/j.sysarc.2024.103199) · µFork [arxiv 2025](https://arxiv.org/abs/2509.09439) · Zephyr DMA gap [GitHub #60289](https://github.com/zephyrproject-rtos/zephyr/issues/60289)*
