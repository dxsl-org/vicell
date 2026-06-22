# Research Learnings & External References
> Part of [Cellos Patterns](../patterns.md)

**Last Updated**: 2026-06-05
**Source**: Adversarial research (108 agents, 26 sources, 5/25 claims verified) + engineering analysis

Những gì Cellos cần học từ các dự án OS khác, kèm nguồn và repo để triển khai lần lượt.

---

## 1. Tock OS — Language-Based Isolation cho MCU

**Tại sao quan trọng**: Closest analogue to Cellos. Tock's capsule ≈ Cellos Cell. Production-deployed (OpenTitan, Chromebook EC).

**Repo**: https://github.com/tock/tock
**Docs**: https://book.tock.os/

### 1.1 Grant Mechanism → Cellos Phase 26

Tock kernel cấp phát memory cho capsule trong *heap của process đó*, không phải kernel heap. Capsule không thể truy cập memory của process khác dù cùng SAS.

```rust
// Tock pattern (học để làm Cellos per-cell memory quota):
// Kernel gọi capsule với grant handle — capsule chỉ thấy phần của mình
fn command(&self, grant: &mut Grant<AppData>) {
    grant.enter(processid, |app, _| {
        app.counter += 1; // chỉ modify data của process này
    })
}
```

**Cellos action (Phase 26)**: Implement `MemoryQuota` từ `GlobalAlloc` caller-PC tracking (spec 02-memory.md §2). Tock source để đọc: `kernel/src/grant.rs`.

### 1.2 Syscall Filter Per Cell → Cellos Phase 27

Mỗi capsule khai báo syscall nào nó được phép dùng. Kernel từ chối tại dispatch time.

```rust
// Tock: capsule registers allowed syscall driver numbers
fn setup_capsules(board: &Board) {
    kernel.add_driver(DRIVER_NUM_UART, uart_capsule);  // only UART syscalls allowed
}
```

**Cellos action (Phase 27)**: Thêm `allowed_syscalls: &[u32]` vào Cell ELF metadata, kernel enforce tại `sys_call_dispatch()`.
**Tock source**: `kernel/src/process.rs`, `capsules/core/`.

### 1.3 Capsule Panic Isolation → Cellos Phase 26

Capsule panic → chỉ kill capsule, kernel + other capsules tiếp tục. Tock dùng `catch_unwind`-equivalent per capsule.

**Cellos action**: Cellos spec (01-core.md) đã có `catch_unwind`. Cần wire vào mọi Cell dispatch path, không chỉ kernel-level panic.
**Tock source**: `kernel/src/process_standard.rs` → `set_process_function()`.

---

## 2. Hubris (Microsoft/Oxide) — IPC Model

**Tại sao quan trọng**: Production Rust RTOS cho RoT (Root of Trust) trong Oxide Computer / Azure Sphere. IPC model gần giống Cellos spec.

**Repo**: https://github.com/oxidecomputer/hubris
**Book**: https://hubris.oxide.computer/

### 2.1 Synchronous Call-Return IPC ("Humility") → Cellos Phase 27

Hubris IPC là **synchronous, blocking, no async queue** — caller blocks until callee returns. Không có race giữa send và receive.

```rust
// Hubris IPC pattern:
// Caller:
let result = sys_send(task_id, operation, data, reply_buf);
// Callee:
let msg = sys_recv(buffer, notification_mask);
sys_reply(caller, result, reply_data);
```

**Cellos relevance**: Cellos hiện dùng `sys_call` (synchronous) và `sys_send`/`sys_recv` (async). Hubris chứng minh synchronous-only IPC đủ cho production và đơn giản hơn nhiều.
**Hubris source**: `sys/kern/src/task.rs` → `send_receive()`.

### 2.2 Lease System (auto-expiring memory loans) → Cellos Phase 27

Khi task A gọi task B, A có thể "lease" memory buffer cho B. Lease tự động expire khi call return — B không giữ được pointer sau đó.

```rust
// Hubris lease pattern (Cellos cần implement tương tự):
// Task A sends message with memory lease:
sys_send(task_b, OP_PROCESS, &data, &mut reply,
         &[Lease::read(&input_buf),   // B can read input
           Lease::write(&mut out_buf)  // B can write output
         ]);
// After sys_send returns: all leases expired — B cannot access buffers
```

**Cellos action**: Thay thế current `CapId` opaque token bằng `Lease` type với auto-revoke. Hubris source: `sys/kern/src/lease.rs`.

---

## 3. RTIC v2 — Priority Scheduler Design

**Tại sao quan trọng**: Static priority + hardware preemption, zero runtime overhead. Pattern cho Cellos Phase 25.

**Repo**: https://github.com/rtic-rs/rtic
**Book**: https://rtic.rs/2/book/en/

### 3.1 Static Priority Assignment → Cellos Phase 25

```rust
// RTIC pattern — Cellos Phase 25 TaskPriority enum nên học theo:
#[rtic::app(device = pac, dispatchers = [SWI0, SWI1, SWI2])]
mod app {
    #[task(priority = 3, binds = UART)]   // RealTime — interrupt-bound
    fn uart_rx(cx: uart_rx::Context) { ... }

    #[task(priority = 2)]                  // Normal — network, shell
    fn tcp_handler(cx: ...) { ... }

    #[task(priority = 1)]                  // Background — bench, LLM
    fn batch_work(cx: ...) { ... }
}
```

**Cellos action**: `TaskPriority { RealTime=0, Normal=1, Background=2 }`. Preempt lower-priority khi higher-priority becomes runnable. RTIC source: `rtic-macros/src/codegen/dispatchers.rs`.

### 3.2 Software Task Dispatching → Cellos Phase 25

RTIC không dùng timer tick cho software tasks — dùng **pending interrupt** để trigger dispatch. Zero overhead khi không có work.

**Cellos action**: Thay vì round-robin timer tick 10ms, dùng RISC-V software interrupt để wake specific priority level khi có task ready. RTIC source: `rtic-sw-pass/src/`.

---

## 4. RustyHermit — libOS & Pure Rust Unikernel

**Verified ✅ 3-0** — confirmed pure Rust, no C/C++, bundles app with kernel.

**Repo**: https://github.com/hermit-os/kernel (kernel library)
**Repo**: https://github.com/hermit-os/hermit-rs (application SDK)
**Website**: https://hermit-os.org/

### 4.1 libOS Pattern (function call, not syscall) → Cellos Phase 27

RustyHermit kernel functions được compile vào app binary — IPC là function call, không phải syscall trap.

```
RustyHermit:          Cellos Phase 27 target:
app calls net_send()  Cell A calls CellB::process()
     ↓                      ↓
direct function call   vtable dispatch (no syscall)
     ↓                      ↓
~3 cycles             ~3 cycles (spec 01-core.md target)
```

**Hermit source để đọc**: `src/syscalls/net.rs` — cách họ implement "syscall" thực ra là function call trong same address space.

### 4.2 SMP Per-CPU Scheduler → Cellos Phase 32

Hermit có per-CPU run queue với work stealing. Relevant khi Cellos cần multi-core support.

**Hermit source**: `src/scheduler/mod.rs`, `src/scheduler/task.rs`.

---

## 5. RedLeaf — Academic Validation của Cellos Thesis

**Verified ✅ 3-0** (USENIX OSDI 2020, peer-reviewed).

**Paper**: https://www.usenix.org/system/files/osdi20-narayanan_vikram.pdf
**Project**: https://mars-research.github.io/redleaf
**Follow-up (limitations)**: https://arkivm.github.io/publications/2021-plos-rust-isolation.pdf

### 5.1 LBI Replaces Hardware MMU — Validated

> *"RedLeaf does not rely on hardware address spaces for isolation and instead uses only type and memory safety of the Rust language."* — OSDI 2020

**Cellos relevance**: RedLeaf = peer-reviewed proof of concept cho Cellos thesis. Khi cần cite academic validation, dùng bài này.

### 5.2 "Isolation in Rust: What is Missing?" (2021 follow-up)

Bài follow-up examine các **limitation** của LBI:
- Unsafe code trong dependencies: `cargo-geiger` không catch transitive unsafe
- `Arc<T>` reference counting có thể leak nếu cycles tồn tại
- Physical side-channels (cache timing) vẫn possible

**Cellos action**: Đọc bài này trước khi thiết kế Phase 26 (memory quota) và Phase 28 (WASM isolation). Source: link ở trên.

---

## 6. Iso-UniK — Hardware-Assisted SAS Isolation

**Verified ✅ 2-1** (Springer Cybersecurity, 2020).

**Paper**: https://link.springer.com/article/10.1186/s42400-020-00051-9

### 6.1 RISC-V ePMP Thay Vì Intel MPK → Cellos Phase 28

Iso-UniK dùng Intel MPK (16 protection domains, ~20 cycles overhead) cho intra-SAS isolation. RISC-V equivalent:

| Mechanism | Platform | Domains | Overhead | Status |
|-----------|----------|---------|----------|--------|
| Intel MPK | x86_64 | 16 | ~20 cycles | Production |
| ARM MTE | AArch64 | 16 tags | ~1 cycle | Production |
| RISC-V ePMP (Smepmp) | RV64 | 16 entries | ~50 cycles | Ratified 2022 |
| RISC-V PMP | RV64 | 16 entries | ~50 cycles | Standard |

**Cellos action (Phase 28)**: Thay vì chờ CHERI, dùng **RISC-V ePMP** để enforce Cell boundaries. Mỗi Cell = 1 PMP entry. Violation → trap → kernel isolate cell.

Iso-UniK "reverse priority isolation" pattern:
```
Standard OS:   kernel = highest privilege, user = lowest
Iso-UniK:      kernel = most restricted PMP entry,
               each cell = own PMP entry
               → kernel cannot accidentally write cell memory
```

**RISC-V spec**: https://github.com/riscv/riscv-tee (Smepmp extension)

---

## 7. Singularity (2003–2009) — IPC Typed Channels

**Source (primary)**: Joe Duffy's blog — https://joeduffyblog.com/2015/11/03/blogging-about-midori/
**Academic paper**: Singularity RDK, SOSP 2007 (Microsoft Research)
**Note**: Claims về Singularity/Midori không pass adversarial verification (single-source blog). Treat as engineering insight, không phải verified fact.

### 7.1 Typed IPC Channels → Cellos Phase 27

Singularity's Software Isolated Processes (SIPs) giao tiếp qua **typed channels**, không phải raw bytes:

```
Singularity (C#-like):
channel FileChannel sends Open(string path) | Read(int len) | Close();
// Compiler enforces protocol order — can't Read before Open

Cellos hiện tại:
[u8; 512] buffer + opcode byte → dễ bị protocol mismatch
```

**Cellos action**: Định nghĩa IPC channel types trong `libs/api/` dùng Rust enums. Compile-time check thứ tự protocol.

```rust
// Cellos typed channel concept (Phase 27):
pub enum VfsRequest {
    Open { path: ArrayString<253> },
    Read { fd: u32, buf_len: u32 },
    Write { fd: u32, data: Box<[u8]> },
    Close { fd: u32 },
}
```

### 7.2 Capability Manifests trong ELF → Cellos Phase 29

Mỗi SIP khai báo capabilities trong manifest trước khi load. Kernel verify tại load time.

**Cellos action**: Thêm `.Cellos_manifest` ELF section với capability declaration:
```toml
# cells/apps/net-tools/manifest.toml (compile-time embed)
[capabilities]
network = true
block_io = false
spawn = false
```
Kernel đọc section này tại `loader.rs:spawn_from_path()` và enforce.

---

## 8. Midori (2008–2015) — Error Model & Async

**Primary source**: Joe Duffy's blog series (2015–2016):
- https://joeduffyblog.com/2015/11/03/blogging-about-midori/
- https://joeduffyblog.com/2016/02/07/the-error-model/
- https://joeduffyblog.com/2015/11/19/asynchronous-everything/
- https://joeduffyblog.com/2015/12/19/safe-native-code/

### 8.1 Escape Hatch — Lesson From Failure → Cellos Phase 28 (Critical)

Midori/Singularity không thể chạy legacy software → zero adoption path → dự án chết.

**Cellos action**: Tier 3 VM (Phase 28) phải implement **trước** khi release Cellos cho external users. Không có VM = không có escape hatch = same fate as Midori.

### 8.2 ZST Capabilities-as-Types → Cellos Phase 26

Midori encode capabilities vào type system — giá trị của type `TcpSocket` **là** capability. Không thể forge vì type system enforce.

Cellos spec (01-core.md) đã mô tả ZST capability tokens — **chưa implement**.

```rust
// Cellos ZST capability pattern (Phase 26) — học từ Midori:
pub struct NetworkCap(());  // ZST — zero runtime cost
pub struct BlockIoCap(());

// Kernel cấp qua Cell init() only:
pub fn cell_init() -> (NetworkCap, /* other caps */) {
    // Only kernel can construct NetworkCap
    (NetworkCap(()), ...)
}

// Usage — compiler rejects if cell doesn't have cap:
pub fn tcp_connect(_cap: &NetworkCap, addr: IpAddr) -> Result<TcpStream>
```

---

## 9. Embassy — Async Executor Pattern

**Repo**: https://github.com/embassy-rs/embassy
**Docs**: https://embassy.dev/

**Relevance**: Embassy's async executor là state-of-the-art cho bare-metal Rust. Cellos's ostd executor nên học từ đây.

### 9.1 Interrupt-Driven Wakers → Cellos smoltcp improvement

Embassy integrate async executor với hardware interrupts — khi IRQ fires, executor wake đúng task đang chờ event đó.

```rust
// Embassy pattern — Cellos network polling nên làm tương tự:
#[embassy_executor::task]
async fn net_task(stack: Stack<'static>) {
    stack.run().await  // woken by VirtIO IRQ, not busy-poll
}
```

**Cellos action**: Thay smoltcp busy-poll loop bằng IRQ-driven waker. Embassy source: `embassy-net/src/lib.rs` → `poll()`.

---

## Tổng Hợp Theo Phase

| Phase | Learn From | Concept | Source |
|-------|-----------|---------|--------|
| **25** | RTIC v2 | Static priority + software interrupt dispatch | [rtic-rs/rtic](https://github.com/rtic-rs/rtic) |
| **26** | Tock OS | Grant mechanism (per-cell memory) | [tock/tock](https://github.com/tock/tock) `kernel/src/grant.rs` |
| **26** | Midori | ZST capabilities-as-types | [Joe Duffy blog](https://joeduffyblog.com/2015/11/03/blogging-about-midori/) |
| **26** | Tock OS | Capsule panic isolation | [tock/tock](https://github.com/tock/tock) `kernel/src/process_standard.rs` |
| **27** | Hubris | Synchronous call-return IPC + lease expiry | [oxidecomputer/hubris](https://github.com/oxidecomputer/hubris) `sys/kern/src/` |
| **27** | RustyHermit | libOS pattern (vtable, not syscall) | [hermit-os/kernel](https://github.com/hermit-os/kernel) `src/syscalls/` |
| **27** | Singularity | Typed IPC channels (enum, not raw bytes) | [SOSP 2007 paper](https://www.microsoft.com/en-us/research/publication/singularity-rethinking-the-software-stack/) |
| **27** | Tock OS | Syscall filter per cell | [tock/tock](https://github.com/tock/tock) `kernel/src/process.rs` |
| **28** | Iso-UniK | RISC-V ePMP for hardware Cell boundaries | [Springer paper](https://link.springer.com/article/10.1186/s42400-020-00051-9) |
| **28** | Midori | Tier 3 VM = escape hatch (no VM = no adoption) | [Joe Duffy blog](https://joeduffyblog.com/2015/11/19/asynchronous-everything/) |
| **29** | Singularity | Cell capability manifests trong ELF `.Cellos_manifest` | [SOSP 2007 paper](https://www.microsoft.com/en-us/research/publication/singularity-rethinking-the-software-stack/) |
| **net** | Embassy | IRQ-driven async waker thay vì smoltcp busy-poll | [embassy-rs/embassy](https://github.com/embassy-rs/embassy) `embassy-net/src/` |
| **32** | RustyHermit | SMP per-CPU scheduler + work stealing | [hermit-os/kernel](https://github.com/hermit-os/kernel) `src/scheduler/` |

## Academic Validation

| Claim | Paper | Venue | Year |
|-------|-------|-------|------|
| LBI thay hardware MMU là viable | [RedLeaf](https://www.usenix.org/system/files/osdi20-narayanan_vikram.pdf) | USENIX OSDI | 2020 |
| LBI limitations (transitive unsafe, side-channel) | [Isolation in Rust: What is Missing?](https://arkivm.github.io/publications/2021-plos-rust-isolation.pdf) | PLOS | 2021 |
| Intra-SAS isolation qua MPK | [Iso-UniK](https://link.springer.com/article/10.1186/s42400-020-00051-9) | Springer Cybersecurity | 2020 |
| Pure Rust unikernel viable (RustyHermit) | [hermit-os.org](https://hermit-os.org/) + [GitHub](https://github.com/hermit-os/kernel) | — | Active 2026 |
