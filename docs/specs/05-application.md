# ViCell Architecture: Application Tiers
**Version**: 0.7 (Drop WASM Tier 2; Minimal VMM for Tier 3)
**Status**: Definitive — updated 2026-06-06 after brainstorm session

---

## 1. Chiến lược phân tầng (The Tiered Strategy)

ViCell phân cấp ứng dụng dựa trên sự cân bằng giữa **Hiệu năng**, **Tính an toàn**, và **Tính tương thích**.

| Đặc điểm | Tier 1: Native | Tier 1b: C Libs | Tier 3: Virtual |
| :--- | :--- | :--- | :--- |
| **Công nghệ** | Rust cells (SAS) | vicell-libc + FFI | Hypervisor Cell |
| **Hiệu năng** | 100% native | 100% native | ~85-90% native |
| **Cách ly** | Compiler (LBI) | Compiler (LBI) | Hardware Stage-2 |
| **Toolchain** | cargo | cargo + cc crate | Linux ecosystem |
| **Trusted** | Bắt buộc | Bắt buộc | Không cần |

**Tier 2 WASM không có trong stack — xem §6 (Wrong Paths) để hiểu lý do.**

---

## 2. Tier 1: Native Cells

Dành cho kernel, drivers, services, RT control — bất cứ thứ gì cần hiệu năng tuyệt đối hoặc quyền truy cập hardware.

- Rust `.o`, chạy trong SAS (Single Address Space)
- Isolation: Rust type system (Language-Based Isolation)
- Bắt buộc: `#![forbid(unsafe_code)]` cho Cells; `unsafe` chỉ trong kernel/HAL
- Không giới hạn file count — full Cargo crate với submodules

---

## 3. Tier 1b: C Library Integration

Dành cho **nhúng thư viện C/C++ vào Rust cell** — link trực tiếp vendor SDK, legacy firmware, hoặc thư viện C không có Rust equivalent mà không cần rewrite.

**Use case chính:**
- Vendor NPU SDK (RKNN, Hailo, K230 KPU) — không có Rust alternative
- Camera ISP library từ silicon vendor
- Validated/certified C codebase (DO-178, IEC 62443) — rewrite phá cert
- Legacy robot firmware C/C++ (10K+ LOC) — rewrite cost quá cao

**Cách hoạt động:** Rust cell link statically với C library. Các lời gọi POSIX bên trong C code (`malloc`, `open`, `read`...) được resolve sang `vicell-libc` (Newlib + POSIX shim) tại link time — chạy native trong SAS, 0ms overhead.

```
[Tier 1b link flow:]
  cell.rs (Rust, owns the cell)
    └── extern "C" { fn rknn_init(...); }   ← FFI bindings
         ↓ links statically
        librknn_api.a  (vendor SDK, C/C++)
         ↓ malloc/open/read → resolve to
        vicell-libc  (libs/api posix feature, Newlib shim)
         ↓ → ViSyscall (VFS IPC, Net IPC, GetTime, GetRandom)
```

**Implementation hiện tại** (`libs/api/src/posix.rs`, 482 lines, feature flag `posix`):

| Nhóm | Functions | Status |
|---|---|---|
| Memory | `malloc/free/realloc/calloc` | ✅ Done (AllocHeader, 16-byte align) |
| Strings | `memcpy/memmove/memset/strlen/strcpy/strcmp` | ✅ Done |
| Files | `_open/_read/_write/_close/_lseek` → ViSyscall | ✅ Done |
| Time | `_time/_gettimeofday` → ViSyscall::GetTime | ✅ Done |
| Exit | `_exit` → ViSyscall::Exit | ✅ Done |
| Entropy | `getentropy/arc4random_buf` → ViSyscall::GetRandom | 🔶 Cần thêm (~50 LOC) |
| Network | `connect/send/recv/close` → Net IPC | 🔶 Cần thêm (~200 LOC) |
| Process | `_fork/_execve/_kill/_wait` | ❌ Returns -1 (SAS incompatible) |
| Memory map | `_sbrk` | ❌ Returns NULL (Rust allocator used) |

**Limitations (by design — không fix):**
- `fork()` = -1 — thư viện C không cần fork; app cần fork → Tier 3
- `mmap(MAP_ANONYMOUS)` = không support — Rust allocator quản lý heap
- Dynamic linking = không support — statically-linked only
- Signals/kill = không support — thư viện C hiếm khi dùng signals

**C libraries phù hợp:**
- ✅ RKNN SDK, Hailo SDK, K230 KPU (NPU inference)
- ✅ mbedTLS, wolfSSL (TLS, sau khi có entropy)
- ✅ SQLite (embedded database)
- ✅ libopus, libvpx (codec, không cần fork)
- ✅ Vendor sensor calibration/fusion libraries
- ❌ Libraries dùng `dlopen` (dynamic plugins)
- ❌ Libraries fork subprocess (libgit2 hooks, ffmpeg filters)

**Tier 1b vs Tier 3b — khi nào dùng cái nào:**

| | Tier 1b: C library link | Tier 3b: Linux VM |
|---|---|---|
| Overhead | 0ms — native SAS | 2-10s boot |
| Isolation | LBI (Rust type system) | Hardware Stage-2 MMU |
| fork/exec | ❌ By design | ✅ Full Linux |
| Phù hợp | Vendor SDK, validated C lib | Full Linux ecosystem, fork-heavy apps |
| Trust requirement | Must be trusted (cùng SAS với kernel) | Untrusted OK (hardware fence) |

---

## 4. Tier 3: Virtualization (Legacy & Security)

### 4.1 Tại sao cần Tier 3

Tier 1 + Tier 1b tốt cho code tin cậy nhưng thiếu ecosystem. G2 target (server/PC) cần:
- nginx, PostgreSQL, Node.js, Python full, Java — không port được hết lên ViCell
- **Giải pháp**: Chạy Linux VM bên trong ViCell như 1 Tier 1 Hypervisor Cell

Analogy: WSL2 trên Windows — chạy Windows + Linux side-by-side, Linux disk/net nối vào Windows.

### 4.2 Hai flavors — khác nhau hoàn toàn

#### Tier 3a — Security Silo [G1-optional]

```
Mục đích: Chạy code cực nhạy cảm (private keys, crypto) trong vùng phần cứng cô lập
Guest: bare-metal Rust binary no_std (~10KB) — không cần OS
Interface: 1 shared memory page (ngoài Stage-2 fence) + notification channel
Boot time: <1ms
```

Use case thực tế G1: robot lưu TLS private key trong Silo — ngay cả kernel ViCell
bị compromise cũng không đọc được key (Stage-2 hardware fence).

Không cần device emulation, không cần Linux. Reuse Stage-2 primitives của Tier 3b.

#### Tier 3b — Linux VM [G2]

```
Mục đích: Chạy Linux ecosystem (apt install nginx → works)
Guest: Linux kernel + userspace, khởi động bình thường
Interface: VirtIO devices (disk, net, console) → forward sang ViCell services
Boot time: 2-10 giây (Linux init)
```

Diagram:
```
ViCell (HS-mode)
├── Tier 1/1b cells (HU-mode) — vfs, net, shell, drivers
└── Hypervisor Cell (Tier 1, HS-mode capable)
    ├── vicell_hv/ (minimal VMM, ~9K LOC Rust)
    │   sys_create_vm / sys_create_vcpu
    │   sys_map_guest_memory → Stage-2 setup
    │   sys_run_vcpu (blocking until VM exit)
    │   sys_vcpu_get/set_regs / sys_inject_irq
    └── VirtIO backends (MMIO bus, no PCI)
        virtio-blk  → sys_send(VFS_ENDPOINT, ...)
        virtio-net  → sys_send(NET_ENDPOINT, ...)
        virtio-console → serial output
        virtio-gpu  → sys_send(COMPOSITOR, ...) [G2+]

    └── Linux Guest (VS-mode, trong Stage-2 fence)
            apt install nginx; nginx; → works
```

### 4.3 VMM: Minimal VMM (custom, ~9K LOC)

**Hypervisor Cell là Tier 1 Rust cell bình thường** — cùng spawn/lifecycle/IPC/restart pattern với vfs/net/shell cells. Điểm khác duy nhất: có `HypervisorCap` capability token, được kernel dùng để gate hypervisor syscalls và switch HS-mode khi dispatch.

**Capability gating** (theo pattern hiện có tại `kernel/src/task/cap.rs` và `tcb.rs:148-153`):
```rust
// Follows same ZST token pattern as BlockIoCap, NetworkCap, SpawnCap
pub struct HypervisorCap;

// In Task struct:
hypervisor_cap: Option<HypervisorCap>,
// syscall_allowlist bitmap gates: sys_create_vm, sys_create_vcpu,
// sys_map_guest_memory, sys_run_vcpu, sys_vcpu_regs, sys_inject_irq
```

**Restart semantics:** Hypervisor Cell chết → NotifyOnExit (204) wakes init → init respawns cell → Linux guest boot lại. Linux RAM state lost (ephemeral), disk state survive qua VirtIO blk → VFS. Identical với cách init restart vfs/net/shell hôm nay.

**IPC pattern (VirtIO backend → ViCell cells):**
```
Linux guest MMIO write (disk I/O)
  → sys_run_vcpu() returns VmExit::MmioWrite
  → Hypervisor Cell: sys_send(VFS_ENDPOINT, read_req)   ← cell-to-cell IPC
  → VFS Cell processes → sys_send(HYPERVISOR_TID, resp)
  → Hypervisor Cell injects VirtIO completion into guest
```

**Multi-instance:** N Hypervisor Cells = N độc lập Linux VMs. Không có gì ngăn spawn nhiều instance — kernel treat chúng như N Tier 1 cells bình thường. Trong G2: thường 1 instance (Option A). Cho isolated workloads: N instances (Option B, Firecracker-style).

ViCell tự viết VMM tối giản thay vì fork crosvm (~75K LOC thực tế, kéo theo tokio + mmap dependencies).

**Thiết kế VMM:**
- Rust-native Tier 1 cell, không có tokio, không mmap, không libc
- Target: `microvm` profile — MMIO bus only, không PCI bus emulation
- VirtIO: `virtio-blk`, `virtio-net`, `virtio-console` over MMIO
- VirtIO backends forward về ViCell IPC (VFS Cell, Net Cell) — không cần implement storage/net stack riêng
- Stage-2 page table: dùng lại primitives từ `kernel/src/memory/`

**Tại sao không fork crosvm:**
- crosvm thực tế ~75K LOC (không phải ~20K như estimate ban đầu)
- Depends tokio (async runtime) + mmap — cả hai không fit SAS cell
- Upstream drift: crosvm thay đổi thường xuyên theo ChromeOS
- microvm profile không cần 90% features của crosvm (VFIO, USB, balloon, etc.)

**Tại sao không QEMU:** ~1M LOC C, cần JIT/mmap/fork — không fit Tier 1 cell.
**Tại sao không Firecracker:** thiếu GPU/display backend — chỉ cho serverless, không G2 desktop.

**Cấu trúc `cells/services/hypervisor/` (~9K LOC):**
```
src/
  vcpu.rs       (~1K) — vCPU struct, run loop, VM exit dispatch
  vm.rs         (~1K) — VM lifecycle, Stage-2 page tables
  mmio.rs       (~500) — MMIO bus, device dispatch
  virtio/
    blk.rs      (~1.5K) — virtio-blk → VFS IPC
    net.rs      (~1.5K) — virtio-net → Net IPC
    console.rs  (~500) — virtio-console → serial
  arch/
    riscv.rs    (~1K) — H-extension vCPU ops
    arm64.rs    (~1K) — EL2 vCPU ops [G2]
    x86.rs      (~1K) — VT-x vCPU ops [G2]
```

### 4.4 Kernel H-extension requirements (RISC-V)

**Privilege mode change khi H-extension detect:**
```
Không có H-ext (hiện tại):   M-mode → S-mode (kernel) → U-mode (cells)
Có H-ext (Tier 3 ready):     M-mode → HS-mode (kernel) → HU-mode (cells)
                                                         → VS/VU-mode (guest)
```

SBI tự detect và delegate vào HS-mode thay vì S-mode khi H-ext có.
Cells chạy HU-mode — transparent, không thay đổi cell code.

**Kernel changes:**
```
hal/arch/riscv/hypervisor.rs  (~200 LOC)
  H-extension detection (misa CSR bit 'H')
  HS-mode boot path (transparent fallback to S-mode if no H-ext)
  New CSRs: hstatus, hgatp, hedeleg, hideleg, hip, hie

kernel/src/hypervisor/         (~800 LOC, new module)
  VM struct + Stage-2 page table management
  vCPU struct + run loop + VM exit dispatch

kernel/src/syscall/hypervisor.rs  (~300 LOC)
  sys_create_vm, sys_create_vcpu, sys_map_guest_memory
  sys_run_vcpu (blocking), sys_vcpu_regs, sys_inject_irq
```

**Không đụng**: scheduler, IPC, memory quota, normal cell lifecycle.

### 4.5 Multi-arch HAL trait

```rust
/// Hardware virtualization interface — one impl per arch.
pub trait ViHypervisor {
    fn detect() -> bool;
    fn create_stage2_table() -> Stage2Table;
    fn map_guest_region(table: &mut Stage2Table, gpa: PAddr, hpa: PAddr, size: usize, flags: MapFlags);
    fn run_vcpu(vcpu: &mut Vcpu) -> VmExit;
    fn inject_irq(vcpu: &mut Vcpu, irq: u32);
}
```

| Arch | Mechanism | Status |
|---|---|---|
| RISC-V | H-extension (HS-mode, hgatp Stage-2) | G1 prep + G2 impl |
| ARM64 | EL2 (HCR_EL2, VTTBR_EL2, Stage-2) | G2 |
| x86_64 | VT-x/AMD-V (VMCS, EPT) | G2 |

### 4.6 Implementation phases

```
G1 prep (khi H-extension đã stable, non-breaking):
  hal/arch/riscv/hypervisor.rs — H-ext detect + HS-mode boot
  → Test: kernel boots identically with/without H-ext
  → Viết hypervisor syscall stubs (return ENOSYS nếu Tier 3 not enabled)

G2 Phase 31+:
  Step 1: Build vicell_hv/ minimal VMM from scratch (~9K LOC)
          vcpu + vm + mmio bus + virtio-blk/net/console
  Step 2: VirtIO backends → ViCell IPC (VFS Cell, Net Cell)
  Step 3: Boot Alpine Linux guest in QEMU (minimal test)
  Step 4: Security Silo implementation (reuse Stage-2 primitives)
  Step 5: ARM64 EL2 + x86 VT-x ports
  Step 6: Android guest (requires GPU passthrough — G2+)
```

---

## 5. Platform Profiles

| Profile | Tiers | Hardware | Use case |
|---|---|---|---|
| **ViCell-Nano** | Tier 1 | RV32, <512KB | MCU, motor/sensor control |
| **ViCell-Standard** | Tier 1 + 1b + 3a | RV64/ARM64 SBC | Robot brain, edge AI |
| **ViCell-Server** | Tier 1 + 1b + 3a + 3b | x86_64 / ARM64 | Server, PC, cloud node |

---

## 6. Những đường sai cần tránh (Wrong Paths)

1. **Type-1 hypervisor**: Tier 3 phải chạy ON TOP of ViCell, không phải thay thế kernel. ViCell kernel = Type-2 host.
2. **Port QEMU**: Quá lớn (~1M LOC C, cần JIT/mmap) — không fit Tier 1 cell.
3. **Fork crosvm**: ~75K LOC thực tế (không phải ~20K), kéo theo tokio + mmap — không fit SAS cell. Build minimal VMM từ scratch (~9K LOC) thay thế.
4. **Gộp Security Silo và Linux VM**: Hai use case khác nhau — implement riêng, reuse Stage-2 primitives.
5. **Assume H-ext mọi nơi**: RV32 không có H-ext. ARM dùng EL2. x86 dùng VT-x. Phải per-arch HAL.
6. **Android G1**: Android cần GPU passthrough + camera HAL + binder IPC — G2+ only, đừng để Android shape G2 design sớm.
7. **WASM Tier 2 (wasmi / WAMR / WASI)**: Semi-trusted zone giả định không tồn tại trong thực tế:
   - G1 (robot/embedded): code đều là trusted Rust — R&D/thử nghiệm diễn ra trên PC không phải thiết bị
   - G2 (server/PC): code trusted → Tier 1 (nhanh hơn 5-10x), code untrusted → Tier 3 VM (isolation mạnh hơn)
   - WASM không có use case rõ ràng nằm giữa hai case trên
   - Phase 28 WASM MVP (wasmi + vi.*) giữ lại dưới `feature = "wasm-experimental"` — không roadmap tiếp
   - Revisit nếu ViCell G2 trở thành multi-tenant platform (third-party workloads từ internet)
8. **WASI Preview 1**: Deprecated (2019 spec), bỏ qua hoàn toàn.
