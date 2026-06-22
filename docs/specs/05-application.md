# Cellos Architecture: Application Tiers
**Version**: 0.9 (Security Silo reclassified from Tier 3a → Tier 1 hardware capability)
**Status**: Definitive — updated 2026-06-19

---

## 1. Chiến lược phân tầng (The Tiered Strategy)

Cellos phân cấp ứng dụng dựa trên sự cân bằng giữa **Hiệu năng**, **Tính an toàn**, và **Tính tương thích**.

| Đặc điểm | Tier 1: Native | Tier 1b: C Libs | Tier 3: Virtual |
| :--- | :--- | :--- | :--- |
| **Công nghệ** | Rust cells (SAS) | Cellos-libc + FFI | Hypervisor Cell |
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

### 2.1 Tier 1 Hardware Extensions (G2 ARM64/x86)

Một số capabilities yêu cầu hardware support nhưng vẫn là **Tier 1 API** — không phải Tier 3, không cần hypervisor.

#### Hardware Key Isolation (Silo)

```
Layer: ostd::silo::SiloHandle (Tier 1 API)
Hardware: ARM64 Stage-2 / x86 VT-x (G2 only — not RISC-V)
Purpose: TLS private keys an toàn ngay cả khi Cellos kernel bị compromise
```

Silo không phải là một VM tier. Nó là một Tier 1 Rust API consume một hardware fence:

```rust
// cells/apps/silo-test/src/main.rs
let handle = ostd::silo::SiloHandle::connect()?;
handle.init_key(&entropy)?;
let sig = handle.sign(&sha256_digest)?;        // P-256 ECDSA
let (our_pub, shared) = handle.ecdh(&peer_pub)?;  // ECDH key agreement
```

Implementation: `silo-guest` binary (~10KB bare-metal AArch64 no_std) chạy trong Stage-2 fenced memory, dispatch bằng mailbox page. Đây là **kernel infrastructure firmware**, không phải app tier.

#### Hardware Supplement to LBI (Planned G2)

LBI (Rust type system) alone có 3 limitations:
1. **rustc là TCB** — compiler correctness là load-bearing; compromised compiler bypass toàn bộ guarantee
2. **Spectre/Meltdown** — shared L1/L2 cache là side channel dù có LBI
3. **Mutable statics** — `static mut` = ambient authority (Midori finding)

Hardware supplement (Tier 1, G2 roadmap — no virtualization):

| Platform | Mechanism | Effect |
|---|---|---|
| ARM64 | MTE (Memory Tagging Extension) | Pointer tags, Spectre mitigation |
| x86 | MPK (Memory Protection Keys) | 16 per-Cell access domains, no TLB flush |
| RISC-V | PMP (Physical Memory Protection) | M-mode fence cho high-value Cells |

---

## 3. Tier 1b: C Library Integration

Dành cho **nhúng thư viện C/C++ vào Rust cell** — link trực tiếp vendor SDK, legacy firmware, hoặc thư viện C không có Rust equivalent mà không cần rewrite.

**Use case chính:**
- Vendor NPU SDK (RKNN, Hailo, K230 KPU) — không có Rust alternative
- Camera ISP library từ silicon vendor
- Validated/certified C codebase (DO-178, IEC 62443) — rewrite phá cert
- Legacy robot firmware C/C++ (10K+ LOC) — rewrite cost quá cao
- Complex C apps: DOOM, FFmpeg, SQLite, mbedTLS (yêu cầu mlibc Tier B)

### 3.1 Two-tier C library strategy (G2)

**Tier A — posix.rs shim** (default, embedded/simple cells, no build overhead):

| | Tier A: posix.rs | Tier B: mlibc |
|---|---|---|
| Binary size | Small | Larger (Grisu3, slab alloc) |
| printf float | Limited | Grisu3 (correct %.15g) |
| malloc | Bump arena | frg::slab_allocator |
| Build | Rust only | WSL2 Meson build first |
| Default | Yes | Opt-in via feature |

**CRITICAL mutual exclusion:** `api = { features = ["mlibc"] }` suppresses posix.rs. Forget the feature while using mlibc-shim → duplicate-symbol link error. **Never link both.**

See `docs/mlibc-build.md` for the full Tier B build guide.

**Cách hoạt động:** Rust cell link statically với C library. Các lời gọi POSIX bên trong C code (`malloc`, `open`, `read`...) được resolve sang `Cellos-libc` (Newlib + POSIX shim) tại link time — chạy native trong SAS, 0ms overhead.

```
[Tier 1b link flow — Tier B mlibc:]
  cell.rs (Rust, owns the cell)
    └── api = { features = ["mlibc"] }  ← posix.rs suppressed
    └── mlibc-shim                      ← links third_party/mlibc/build/libc.a
    └── extern "C" { fn rknn_init(...); }   ← FFI bindings
         ↓ links statically
        librknn_api.a  (vendor SDK, C/C++)
         ↓ malloc/open/read → resolve to
        libc.a  (mlibc Tier B — sysdeps/Cellos → Cellos_syscall)
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

## 4. Tier 3: Virtualization (Linux Ecosystem)

### 4.1 Tại sao cần Tier 3

Tier 1 + Tier 1b tốt cho code tin cậy nhưng thiếu ecosystem. G2 target (server/PC) cần:
- nginx, PostgreSQL, Node.js, Python full, Java — không port được hết lên Cellos
- **Giải pháp**: Chạy Linux VM bên trong Cellos như 1 Tier 1 Hypervisor Cell

Analogy: WSL2 trên Windows — chạy Windows + Linux side-by-side, Linux disk/net nối vào Windows.

> **Note**: Security Silo đã được reclassify sang §2.1 (Tier 1 Hardware Extensions). Silo KHÔNG phải Tier 3 — nó là Tier 1 API không cần hypervisor, không phải VM tier.

### 4.2 Tier 3b — Linux VM [G2]

```
Mục đích: Chạy Linux ecosystem (apt install nginx → works)
Guest: Linux kernel + userspace, khởi động bình thường
Interface: VirtIO devices (disk, net, console) → forward sang Cellos services
Boot time: 2-10 giây (Linux init)
```

Diagram:
```
Cellos (HS-mode)
├── Tier 1/1b cells (HU-mode) — vfs, net, shell, drivers
└── Hypervisor Cell (Tier 1, HS-mode capable)
    ├── Cellos_hv/ (minimal VMM, ~9K LOC Rust)
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

**IPC pattern (VirtIO backend → Cellos cells):**
```
Linux guest MMIO write (disk I/O)
  → sys_run_vcpu() returns VmExit::MmioWrite
  → Hypervisor Cell: sys_send(VFS_ENDPOINT, read_req)   ← cell-to-cell IPC
  → VFS Cell processes → sys_send(HYPERVISOR_TID, resp)
  → Hypervisor Cell injects VirtIO completion into guest
```

**Multi-instance:** N Hypervisor Cells = N độc lập Linux VMs. Không có gì ngăn spawn nhiều instance — kernel treat chúng như N Tier 1 cells bình thường. Trong G2: thường 1 instance (Option A). Cho isolated workloads: N instances (Option B, Firecracker-style).

Cellos tự viết VMM tối giản thay vì fork crosvm (~75K LOC thực tế, kéo theo tokio + mmap dependencies).

**Thiết kế VMM:**
- Rust-native Tier 1 cell, không có tokio, không mmap, không libc
- Target: `microvm` profile — MMIO bus only, không PCI bus emulation
- VirtIO: `virtio-blk`, `virtio-net`, `virtio-console` over MMIO
- VirtIO backends forward về Cellos IPC (VFS Cell, Net Cell) — không cần implement storage/net stack riêng
- Stage-2 page table: dùng lại primitives từ `kernel/src/memory/`

**Tại sao không fork crosvm:**
- crosvm thực tế ~75K LOC (không phải ~20K như estimate ban đầu)
- Depends tokio (async runtime) + mmap — cả hai không fit SAS cell
- Upstream drift: crosvm thay đổi thường xuyên theo ChromeOS
- microvm profile không cần 90% features của crosvm (VFIO, USB, balloon, etc.)

**Tại sao không QEMU:** ~1M LOC C, cần JIT/mmap/fork — không fit Tier 1 cell.
**Tại sao không Firecracker:** thiếu GPU/display backend — chỉ cho serverless, không G2 desktop.

**Cấu trúc `cells/services/hypervisor/` (shipped, ~9K LOC):**
```
src/
  run_loop.rs       — VmExit dispatch loop (MMIO/HVC/WFI/Preempted/Shutdown)
  vmm.rs            — create_vm / create_vcpu / map_guest / run_vcpu wrappers
  loader_image.rs   — ARM64 Image header parser + guest RAM placement
  dtb.rs            — FDT builder (9 nodes: RAM/CPU/PSCI/GIC/timer/chosen/UART/virtio×3)
  pl011.rs          — PL011 UART emulator
  gicd.rs           — GICv2 GICD shadow-register emulator
  psci.rs           — PSCI 1.0 handler (SYSTEM_OFF/CPU_ON/…)
  timer.rs          — armv8-timer virtual IRQ injection
  virtio_mmio.rs    — virtio-mmio transport (QueueNotify, feature negotiation)
  virtqueue.rs      — split virtqueue (avail/used ring, descriptor chain walk)
  virtio_console.rs — virtio-console (slot 0, SPI 16)
  virtio_blk.rs     — virtio-blk → VFS IPC (slot 1, SPI 17)
  virtio_net.rs     — virtio-net → Net IPC, MAC demux (slot 2, SPI 18)
  net_backend.rs    — L2Send/L2Recv IPC helpers to Net Cell
  vgic.rs           — GICH/GICV hardware vGIC (Phase 09)
  loader_image.rs   — guest image placement helper
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
/// Hardware virtualization interface — one impl per arch (hal/traits/hypervisor/).
pub trait ViHypervisor {
    type Vm; type Vcpu; type Stage2Table;
    fn create_vm(&self) -> ViResult<Self::Vm>;
    fn create_vcpu(&self, vm: &mut Self::Vm) -> ViResult<Self::Vcpu>;
    fn map_guest(&self, table: &mut Self::Stage2Table,
                 ipa: u64, hpa: u64, pages: usize, writable: bool) -> ViResult<()>;
    fn run_vcpu(&self, vcpu: &mut Self::Vcpu) -> ViResult<ViVmExit>;
    fn inject_irq(&self, vcpu: &mut Self::Vcpu, intid: u32) -> ViResult<()>;
}
```

| Arch | Mechanism | HAL crate | Status |
|---|---|---|---|
| **ARM64** | EL2 non-VHE (HCR_EL2, VTTBR_EL2, Stage-2, GICH) | `hal-arm` | **✅ G1 shipped** (P01–P10) |
| RISC-V | H-extension (HS-mode, hgatp Stage-2) | `hal-riscv` (ENOSYS stub) | ⏳ G2 — H-ext absent on current boards |
| x86_64 | VT-x (VMCS, EPT) | `hal-x86` (ENOSYS stub) | ⏳ G2 |

Kernel syscall dispatch (`kernel/src/hypervisor/registry.rs`) is `#[cfg(target_arch = "aarch64")]` for the real impl and returns `NotSupported` on riscv64/x86_64 — matching the HAL stubs. No kernel change needed when future RISC-V/x86 impls land.

### 4.6 Implementation status

**ARM64 EL2 VMM — ✅ COMPLETE (G1, 2026-06-16)**
```
Phases 01–10 shipped in cells/services/hypervisor/:
  P01: HAL ViHypervisor trait + ARM64 stay-at-EL2 boot + EL2 MMU/vectors
  P02: Stage-2 builder + guest-RAM carve (128 MiB) + VTTBR/VTCR
  P03: vCPU world-switch + trap decode + bare-metal guest smoke
  P04: Syscalls 220-227 (CreateVm/CreateVcpu/MapGuest/RunVcpu/VcpuRegs/InjectIrq/WriteGuest/ReadGuest)
  P05: Hypervisor cell: guest-load + DTB + PSCI + PL011 + GICD emul → BOOTS ALPINE
  P06: virtio-mmio transport + split virtqueue + virtio-console
  P07: virtio-blk → VFS Cell → mounts rootfs
  P08: virtio-net → Net Cell (L2 MAC-bridge, DHCP → 10.0.2.15, apt works)
  P09: Full GICH/GICV hardware vGIC upgrade (IRQ throughput)
  P10: CI smoke + ENOSYS stubs (riscv64/x86_64) + this docs update
```

**RISC-V H-extension — ⏳ Pending**
```
Current RISC-V boards (SG2042, SG2044, K230) lack H-extension.
ENOSYS stubs in hal-riscv/src/hypervisor.rs + registry.rs are in place.
Impl unblocks when H-ext hardware is available.
```

**x86_64 VT-x — ⏳ Pending (G2)**
```
ENOSYS stubs in hal-x86/src/hypervisor.rs + registry.rs are in place.
VT-x impl deferred to G2.
```

---

## 5. Platform Profiles

| Profile | Tiers | Hardware | Use case |
|---|---|---|---|
| **Cellos-Nano** | Tier 1 | RV32, <512KB | MCU, motor/sensor control |
| **Cellos-Standard** | Tier 1 + 1b + 3a | RV64/ARM64 SBC | Robot brain, edge AI |
| **Cellos-Server** | Tier 1 + 1b + 3a + 3b | x86_64 / ARM64 | Server, PC, cloud node |

---

## 6. Những đường sai cần tránh (Wrong Paths)

1. **Type-1 hypervisor**: Tier 3 phải chạy ON TOP of Cellos, không phải thay thế kernel. Cellos kernel = Type-2 host. ✅ Xác nhận: hypervisor cell là Tier 1 cell bình thường với HypervisorCap.
2. **Port QEMU**: Quá lớn (~1M LOC C, cần JIT/mmap) — không fit Tier 1 cell.
3. **Fork crosvm**: ~75K LOC thực tế (không phải ~20K), kéo theo tokio + mmap — không fit SAS cell. ✅ Build minimal VMM từ scratch (~9K LOC) — đã shipped ARM64 EL2 (P01-P10).
4. **Gộp Security Silo và Linux VM**: Hai use case khác nhau — implement riêng, reuse Stage-2 primitives.
5. **Assume H-ext mọi nơi**: RV32 không có H-ext. ARM dùng EL2. x86 dùng VT-x. Phải per-arch HAL. ✅ ENOSYS stubs cho riscv64/x86_64 landed P10; ARM64 EL2 shipped.
6. **Android G1**: Android cần GPU passthrough + camera HAL + binder IPC — G2+ only, đừng để Android shape G2 design sớm.
7. **WASM Tier 2 (wasmi / WAMR / WASI)**: Semi-trusted zone giả định không tồn tại trong thực tế:
   - G1 (robot/embedded): code đều là trusted Rust — R&D/thử nghiệm diễn ra trên PC không phải thiết bị
   - G2 (server/PC): code trusted → Tier 1 (nhanh hơn 5-10x), code untrusted → Tier 3 VM (isolation mạnh hơn)
   - WASM không có use case rõ ràng nằm giữa hai case trên
   - Phase 28 WASM MVP (wasmi + vi.*) giữ lại dưới `feature = "wasm-experimental"` — không roadmap tiếp
   - Revisit nếu Cellos G2 trở thành multi-tenant platform (third-party workloads từ internet)
8. **WASI Preview 1**: Deprecated (2019 spec), bỏ qua hoàn toàn.
