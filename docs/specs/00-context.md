# Cellos system context & design rules
**Last Updated**: 2026-06-22
**Audience**: Developers & AI Agents


## 🔴 PRIME DIRECTIVE
**Cellos uses Cellular SAS (Single Address Space) + Language-Based Isolation (LBI)**

- ❌ **NOT** traditional Linux/Unix process-based thinking
- ✅ **YES** Cellular architecture with zero-copy IPC
- ✅ **YES** Rust type system for safety, not hardware MMU

**Key Philosophy**: Software organized as **Cells** (not processes), sharing one address space, isolated by Rust's type system.


## 1. Bản đồ tri thức (Source of Truth)
Trước khi code bất kỳ module nào, Agent **BẮT BUỘC** phải đọc file đặc tả tương ứng:
| Nếu nhiệm vụ liên quan đến... | Hãy đọc file này |
| --- | --- |
| Lấy code từ các dự án khác | `design/00-fork.md` |
| Triết lý Cellular & Linker Linker | `design/01-core.md` |
| SAS Layout, HHDM & Metadata Registry | `design/02-memory.md` |
| Async Safety & Owned Buffers Rule | `design/03-runtime.md` |
| Multi-Arch Trait (RV32/64/128) | `design/04-hardware.md` |
| Native vs WASM vs Virtualization | `design/05-application.md` |
| Zero-copy Compositing & Input | `design/06-graphics.md` |
| User-space Stack (smoltcp) | `design/07-networking.md` |
| Tickless Idle & Pointer Swizzling | `design/08-power.md` |
| Pluggable FS & Direct I/O | `design/09-vfs.md` |
| KUnit & Fault Injection | `design/10-testing.md` |


## 2. Cấu trúc thư mục chuẩn
```text
Cellos/
├── kernel/                   # Nano Kernel (Runtime Linker & Manager)
│   └── src/ 
│       ├── boot/             # Khởi tạo sơ khai (Handover từ OpenSBI/Limine) 
│       ├── cell/             # LINH HỒN: Quản lý Metadata, Registry, Dependency
│       ├── loader/           # TRÁI TIM: ELF Linker, vá địa chỉ (Relocation)
│       ├── memory/           # Quản lý Global Heap & Paging (Nền tảng SAS)
│       └── task/             # Executor cho Async Tasks (Không quản lý Process cũ)
│           ├── mod.rs        # Quản lý danh sách Task toàn cục (Task Registry)
│           ├── tcb.rs        # Định nghĩa cấu trúc Task (Registers, Stack, CellOwner)
│           ├── stack.rs      # Quản lý vùng nhớ Stack cho từng Task (kèm Guard Pages)
│           └── scheduler.rs  # Thuật toán điều phối
├── hal/                      # Tầng trừu tượng phần cứng (Arch, Irq, Timer)
│   ├── core/                 # Glue Code (Re-exports traits & Arch definition)
│   ├── arch/                 # THẾ GIỚI CỦA CPU
│   │   ├── riscv/            # Họ RISC-V
│   │   │   └── src/
│   │   │       ├── common/   # Code dùng chung cho cả RV32 và RV64 (PLIC, CLINT, v.v.)
│   │   │       ├── rv64/     # Thực thi cụ thể cho 64-bit (Sv39 paging)
│   │   │       └── rv32/     # Thực thi cụ thể cho 32-bit (cho robot nano, Sv32)
│   │   ├── arm/              # Họ ARM
│   │   │   └── src/
│   │   │       ├── common/   # GIC, Generic Timer dùng chung cho ARM
│   │   │       ├── aarch64/  # ARM 64-bit
│   │   │       └── aarch32/  # ARM 32-bit
│   │   └── x86/              # Họ x86
│   │       └── src/
│   │           ├── common/   # APIC, IOAPIC dùng chung
│   │           └── x86_64/   # Long mode 64-bit
│   └── traits/               # THẾ GIỚI CỦA GIAO DIỆN (HỢP ĐỒNG)
│       ├── uart/             # SerialPort trait (pure interface)
│       ├── display/          # Framebuffer trait (pure interface)
│       ├── timer/            # Timer trait (extracted)
│       └── interrupt/        # InterruptController trait (extracted)
├── cells/                    # Các đơn vị phần mềm độc lập (.o files)
│   ├── apps/                 # Ứng dụng người dùng (Tier 1/2/3)
│   │   ├── init/             #
│   │   └── shell/            #
│   ├── drivers/              # Rust Drivers (Tier 1), C/C++ Drivers (Tier 2)
│   │   ├── disk/             #
│   │   ├── gpu/              #
│   │   ├── input/            #
│   │   ├── net/              #
│   │   ├── serial/           #
│   │   └── wasm/             #
│   ├── runtimes/             # Language Runtimes (Lua, MicroPython)
│   │   ├── micropython/      #
│   │   └── lua/              #
│   └── services/             # Các dịch vụ hệ thống (Tier 1)
│       ├── compositor/       #
│       ├── config/           #
│       ├── input/            #
│       ├── net/              #
│       ├── power/            #
│       └── vfs/              #
├── libs/
│   ├── api/                  # Định nghĩa Trait (FileSystem, TcpStack...)
│   ├── types/                # Các kiểu dữ liệu cốt lõi (CellId, Error...)
│   └── ostd/                 # Thư viện chuẩn dành cho Cells
└── tests/
```

## 3. The Coding Laws
**Luật 1: Interface là "Thánh chỉ"**
- Mọi thay đổi trong libs/api hoặc libs/types phải XÁC NHẬN 2 LẦN với User.
- Các Trait trong api phải dùng #[repr(C)] để bảo đảm Stable ABI.

**Luật 2: An toàn bộ nhớ SAS**
- Owned Buffers ONLY: Cấm truyền &mut [u8] qua ranh giới Async.
- Sử dụng Box<[u8]> để chuyển quyền sở hữu giữa các Cell.

**Luật 3: Đa kiến trúc (Multi-Arch)**
- Code trong kernel và libs/ostd không được giả định 32 hay 64-bit.
- Sử dụng các kiểu dữ liệu từ libs/types và hal/core.

**Luật 4: Quản lý Unsafe**
- **Cells**: #![forbid(unsafe_code)].
- **Kernel/HAL**: Chỉ dùng unsafe khi tương tác trực tiếp phần cứng và phải có # Safety documentation.

**Luật 5: Module Style (Modern Rust)**
- **TUYỆT ĐỐI CẤM** sử dụng `mod.rs`.
- Sử dụng cấu trúc hiện đại: `foo.rs` nằm ngang hàng với thư mục `foo/`.
- Tên file/thư mục phải là snake_case.

**Luật 6: Cellos Naming Convention**
- Mọi thành phần trong mã nguồn phải tuân thủ quy tắc đặt tên để phân biệt giữa **Hợp đồng Cellos (Contract)**, **Thực thi (Implementation)** và **Mã nguồn Fork**.
- **Chống đụng độ:** Không được đặt tên trùng với các thư viện fork (ví dụ: cấm đặt tên Trait là `FileSystem` nếu đã fork RedoxFS).
- **Định danh SAS:** Các Trait bắt đầu bằng `Vi` phải hỗ trợ cơ chế chuyển giao sở hữu (Ownership) hoặc Lease/Grant để tối ưu hóa Single Address Space.
- **Refactor hàng Fork:** Khi đưa mã nguồn ngoại lai vào, phải giữ nguyên logic gốc nhưng phải bọc (wrap) hoặc implement lại các Trait theo chuẩn `Vi` để Kernel có thể gọi.

| Đối tượng | Tiền tố / Quy tắc | Ví dụ |
| --- | --- | --- |
| **Public Trait (ABI)** | **Vi** + PascalCase | `ViFileSystem`, `ViFile`, `ViDriver` |
| **Core Types / Errors** | **Vi** + PascalCase | `ViResult`, `ViError`, `ViConfig` |
| **Hệ thống tập tin** | **vi** + Name + Version | `viFS1` (Redox), `viFS2` (TFS) |
| **Địa chỉ (Multi-Arch)** | **VAddr / PAddr** | `VAddr`, `PAddr` (Bắt buộc dùng từ `libs/types`) |
| **Internal Modules** | **snake_case** | `task/tcb.rs`, `memory/paging.rs` |


**Luật 7: Quản lý Trait & Tính Linh Hoạt (Polymorphism)**
- **Trait Object**: Sử dụng `dyn Trait` thay cho Generics tại các interface hệ thống (VFS, Drivers, Network) để hỗ trợ nạp/gỡ Cell động.
- **Bắt buộc Bounds an toàn**: Mọi trait object được lưu trữ hoặc chuyển giao giữa các Task phải chỉ định rõ Send + Sync (ví dụ: `Arc<dyn ViDriver + Send + Sync>`).
- **Box vs Arc**: Sử dụng `Box` cho các đối tượng có chủ sở hữu duy nhất và Arc cho các tài nguyên dùng chung trong SAS.


**Luật 8: Quản lý tài nguyên & LBI (Memory Safety)**
- **Tối ưu hóa tham chiếu (Borrowing over Heap)**: Sử dụng tham chiếu (`&` / `&mut`) thay vì Ownership (`Box`) khi chỉ cần truy cập dữ liệu tạm thời để giảm áp lực cấp phát lên Global Heap.
- **Lifetime**: Mọi struct chứa tham chiếu phải chỉ định rõ `lifetime` để ngăn chặn dangling references trong Single Address Space.
- **Drop**: Bắt buộc triển khai `Drop` trait cho các cấu trúc quản lý tài nguyên (Lease, FileHandle, DriverContext) để đảm bảo thu hồi tài nguyên ngay lập tức khi đối tượng ra khỏi phạm vi (scope).


## 4. Agent Workflow
- **Check Spec**: Đọc file trong design/ để hiểu "Tại sao".
- **Interface First**: Định nghĩa Trait trong libs/api trước khi code phần thực thi.
- **Thực thi**: Code logic, chú ý xử lý Result thay vì panic! để hỗ trợ Panic Recovery.
- **Verification**: Viết test KUnit cho mọi logic quan trọng.


## 5. Trusted Computing Base (TCB)

Cellos dùng **Language-Based Isolation (LBI)** làm cơ chế bảo mật chính thay cho hardware MMU per-process. Điều này có một hệ quả quan trọng: **`rustc` là load-bearing TCB**.

### Tại sao `rustc` là TCB

- `#![forbid(unsafe_code)]` trên mọi Cell được enforce bởi `rustc` tại compile time — không phải hardware.
- Borrow checker ngăn một Cell giữ live reference vào vùng nhớ của Cell khác — đây là ranh giới cách ly chính.
- Nếu `rustc` bị compromise (compiler supply-chain attack hoặc soundness hole), toàn bộ Cell isolation guarantee sụp đổ.

**Prior art:** Microsoft Singularity/Midori xác định CLR/.NET runtime là load-bearing TCB của họ với cùng pattern. Cellos thay CLR bằng `rustc` — đạt cùng guarantee nhưng không cần GC. Xem `docs/research/research-singularity-midori.md`.

> *"One interesting aspect of relying on type safety was that your compiler becomes part of your TCB."*
> — Joe Duffy, Safe Native Code (2015)

### Tại sao LBI thay vì hardware MMU per-Cell (số đo)

Singularity (MSPC 2006) đo trực tiếp:

| Isolation mechanism | Overhead |
|---|---|
| SIP (Language-Based, Ring 0, no mode-switch) | **<5%** |
| Hardware isolation (added on top) | **25–33%** avg, **37.7%** trên WebFiles macro-benchmark |

Cellos sẽ không dùng per-Cell SATP vì lý do này. Các Cell chia sẻ address space, `rustc` enforce ranh giới tại compile time.

**IPC cost comparison:**

| System | IPC cost |
|---|---|
| Singularity channel (pointer hand-off) | ~1,200 cycles |
| Cellos vtable dispatch | **~2–3 cycles** (400× rẻ hơn) |
| Linux syscall | ~80–150 cycles |

Cellos vtable IPC rẻ hơn Singularity 2 orders of magnitude vì không có channel/heap indirection. Grant API (syscalls 208–212) là analogue của exchange heap cho large data (>page).

### Thành phần TCB (nhỏ nhất → lớn nhất)

| Component | Vai trò | Ước lượng |
|---|---|---|
| `rustc` (nightly Rust compiler) | Enforce LBI — forbid unsafe, ownership/borrow rules | ~3–5M LOC |
| Cellos kernel | SAS memory allocator, scheduler, IPC, ELF loader | ~11.5K LOC |
| `libs/api` + `libs/types` | Stable ABI giữa kernel và Cells | ~2K LOC |

### Không nằm trong TCB (by design)

- **Cell code** — `#![forbid(unsafe_code)]` nghĩa là kernel không trust Cell code; violation bị caught bởi rustc trước khi link.
- **C/C++ thư viện qua Tier 1b FFI** — được isolated trong caller's address space; kernel validate tất cả grant/IPC boundaries.
- **Lua/VM guests (Tier 1b/3)** — sandbox bởi interpreter manifest restrictions hoặc hypervisor Stage-2 paging.

### Mitigation: giảm thiểu rủi ro rustc-as-TCB

Cellos tốt hơn Singularity/Midori ở điểm này vì ba lý do:

1. **rustc là open-source** — cộng đồng lớn, thường xuyên audit; Bartok (Singularity/Midori) là closed-source.
2. **Ferrocene** — ISO 26262 ASIL-D certified subset của rustc; đủ điều kiện cho automotive safety market.
   - ⚠️ **Caveat**: RISC-V chưa là qualified target của Ferrocene (tính đến 2026-06, ETA 12–24 tháng). ARM64 và x86 đã qualified. Không dùng RISC-V safety claim trước khi qualify.
3. **miri** — interprets MIR bytecode, phát hiện unsoundness trong unsafe code. Chạy trong CI cho kernel unsafe paths.

### Policies từ LBI/TCB (phải follow)

| Policy | Rationale |
|---|---|
| **Không link GC runtime vào RT-critical Cells** | Midori's unsolved problem — GC pauses phá RT guarantees. Rust RAII = deterministic destruction là lợi thế cấu trúc quyết định. |
| **`static mut` trong kernel chỉ qua `Spinlock<Option<T>>`** | Mutable statics = "ambient authority" (F6, Duffy). Cells đã block qua `forbid(unsafe_code)`. Kernel là gap còn lại. |
| **`#![forbid(unsafe_code)]` tuyệt đối trên mọi Cell** | Compiler không thể verify safety nếu unsafe được phép — một unsafe line đủ để vô hiệu hóa toàn bộ LBI guarantee. |
| **Upgrade `rustc` ngay khi CVE soundness hole** | Soundness hole trong borrow checker hoặc codegen = P0 breach — không có patch nào khác ngoài upgrade compiler. |

### Hệ quả với security review

Bất kỳ lỗ hổng nào trong `rustc`'s unsafe checker, borrow checker, hoặc codegen cho phép unsafe code trong crate `#![forbid(unsafe_code)]` là **P0 kernel security breach** với Cellos — cần upgrade compiler ngay lập tức.

**Khi review Cell code**, không cần audit memory safety (rustc đã làm). Tập trung vào:
- Logic bugs và incorrect IPC protocol usage
- Grant misuse (phải kiểm tra `!Copy + !Clone` invariant trên owner handle)
- Capability escalation qua IPC (Cell request syscall không có trong manifest)

*Sources: Hunt & Larus ACM 2007; Deconstructing Process Isolation MSPC 2006; Joe Duffy "Safe Native Code" (2015). Full analysis: `docs/research/research-singularity-midori.md`*