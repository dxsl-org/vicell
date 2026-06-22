# Cellos System Architecture

**Audience**: Developers new to Cellos  
**Level**: High-level (conceptual + key components)  
**Version**: 0.2.1-dev (Mycelium Era)  
**Last Updated**: 2026-06-05 (Phase X-3 complete)

---

## Core Philosophy

Cellos is **NOT** a traditional Linux-style OS. It uses:

- **Cellular Architecture**: Software organized as **Cells** (not processes), all sharing one address space
- **Language-Based Isolation**: Rust's type system (not hardware MMU) provides isolation
- **Single Address Space (SAS)**: Kernel and all Cells live in one virtual memory space, with no process boundaries
- **Zero-Copy IPC**: Capability-based message passing using owned buffers

**Impact**: No expensive context switches, no TLB flushes, minimal privilege escalation overhead.

---

## System Layers

```
┌─────────────────────────────────────────┐
│  Cells (Applications, Drivers, Services) │  Apps: hello, shell, lua, micropython
├──────────────────────────────────────────┤  Drivers: disk, gpu, input, net, serial
│  Kernel (Nano Kernel, ~8,700 LOC)       │  Services: vfs, config, compositor, net, power
├──────────────────────────────────────────┤
│  HAL (Hardware Abstraction Layer)        │  RV64 ✅, AArch64 ✅ (Ring-3), x86_64 ✅ (Ring-3)
├──────────────────────────────────────────┤
│  Hardware (QEMU, Bare-metal)             │  Memory, CPU, Devices
└─────────────────────────────────────────┘
```

---

## Kernel (nano-kernel, ~8,700 LOC)

The kernel is **tiny** by design, handling only:

### 1. **Boot & Initialization** (`kernel/src/boot.rs`)
- Limine bootloader integration (fallback: SimpleBootInfo)
- Parse DTB (device tree)
- Initialize UART for logging
- Initialize HAL (interrupts, paging)
- Set up frame allocator
- Initialize memory (paging, heap)
- Initialize scheduler
- Spawn init Cell
- Enable interrupts and enter idle loop

### 2. **Memory Management** (`kernel/src/memory/`)

**Frame Allocator**:
- Bitmap-based allocation (O(1) free, O(n) scan for allocate)
- 128–256 MB physical RAM in QEMU (0x8000_0000–0x8000_0000 + size)
- Tracks allocated vs. free pages (4KB each)

**Virtual Memory (SV39 on RV64)**:
- **Trap Zone**: Low 4KB, unmapped → catches NULL deref
- **User VA**: < 0x8000_0000 (per-task isolation via page tables)
- **Guard Hole**: 0x8000_0000–0x8020_0000 (unmapped, prevents overflow)
- **Kernel VA**: 0x8020_0000+ (identity-mapped)
- **Heap**: 64 MB kernel heap (linked-list allocator)

**Paging Structure** (RV39):
```
User Space: 1 GB (virt addr < 0x8000_0000)
├─ Stack: top of user VA (grows down)
├─ Heap: dynamic (grows up)
└─ Code/Data: loaded from ELF

Kernel Space: (virt addr 0x8020_0000+)
├─ Code: kernel binary
├─ Data: statics, globals
├─ Heap: kernel allocations
└─ Page Tables: per-task
```

### 3. **Task Scheduler** (`kernel/src/task/scheduler.rs`)

**Round-Robin with Time Slices**:
- All Cells scheduled fairly
- Each gets ~10ms time slice (configurable)
- Yield/preempt on timer interrupt

> **Roadmap**: Round-robin 10 ms timeslice is the current baseline. Phase 25 will add three priority levels (RealTime / Normal / Background) to prevent Tier 1 robot-control tasks from being preempted by batch workloads.

**Task Control Block (TCB)**:
```rust
struct Task {
    id: TaskId,
    state: TaskState,          // Running, Ready, Blocked, Dead
    cpu_context: TrapFrame,    // Registers, PC, SP
    page_table: PageTable,     // Task's virtual memory
    parent: TaskId,            // Parent Cell for tracking
    ipc_queue: Queue<Message>, // Incoming IPC messages
    grants: Vec<Grant>,        // Capability objects
}
```

**States**:
- `Running` — executing on CPU
- `Ready` — waiting for CPU
- `Blocked` — waiting for IPC message or I/O
- `Dead` — finished, pending cleanup

### 4. **IPC System** (`kernel/src/task/ipc.rs`)

10 core syscalls (vs. Linux's 300+):

> **Implementation Note**: The architecture spec (01-core.md) describes inter-cell IPC as direct function calls via vtable (2–3 CPU cycles). The current implementation uses kernel-mediated syscall message passing (~100–1000 cycles per round-trip), equivalent to a lightweight microkernel. Direct vtable IPC is planned for Phase 27 (trusted-cell fast path) once the Metadata Registry is integrated with the linker.

| Syscall | Purpose |
|---------|---------|
| `Send(to, msg, cap)` | Send message to Cell, optionally grant capability |
| `Recv(from_filter, timeout)` | Receive message (blocks if none) |
| `Call(to, msg, cap)` | Send + wait for reply (RPC) |
| `Reply(to, msg)` | Reply to caller |
| `Spawn(binary, argv)` | Create new Cell |
| `Exec(binary, argv)` | Replace self with new Cell |
| `SpawnFromMem(ptr, size)` | Load Cell from memory buffer |
| `Exit(code)` | Terminate self |
| `Yield()` | Voluntarily yield CPU |
| `Log(msg)` | Print to kernel log |

**Capability-Based Access Control**:
```rust
pub struct Capability {
    rights: u32,  // Read, Write, Execute, etc.
    target: CellId,
}

pub struct Grant {
    cap: Capability,
    from_cell: CellId,
    to_cell: CellId,
    // Revoked on drop
}
```

### 5. **ELF Loader** (`kernel/src/loader.rs`)

- Parse ELF header
- Load segments (allocate frames, map to vaddr)
- Apply relocations (position-independent code)
- Set up stack, heap pointers
- Enter user-space at `_start`

### 6. **Filesystem (FAT32)** (`kernel/src/fs/`)

- Read-only FAT32 parser for boot
- Contains: `/bin/shell`, `/bin/hello`, `/bin/lua`, `/bin/cat`, `/bin/ls`
- Kernel uses this to spawn init Cell

---

## Hardware Abstraction Layer (HAL)

### Traits (Pure Interfaces)

```rust
// hal/traits/arch/lib.rs
pub trait Arch {
    fn init();
    fn switch_context(old: &TrapFrame, new: &TrapFrame);
    fn enable_interrupts();
    fn disable_interrupts();
}

// hal/traits/paging/lib.rs
pub trait PageTableTrait {
    fn map(&mut self, va: VAddr, pa: PAddr, flags: u32);
    fn unmap(&mut self, va: VAddr);
    fn translate(&self, va: VAddr) -> Option<PAddr>;
}

// hal/traits/interrupt/lib.rs
pub trait InterruptController {
    fn init();
    fn enable_irq(irq: u32);
    fn disable_irq(irq: u32);
    fn ack_irq(irq: u32);
}
```

### Implementations

**RISC-V 64-bit (RV64) — FULLY IMPLEMENTED** ✅
- `hal/arch/riscv/src/rv64/context.rs` — Trap frame, context switch
- `hal/arch/riscv/src/rv64/paging.rs` — SV39 page table walker
- `hal/arch/riscv/src/rv64/trap.rs` — Exception/interrupt handler
- `hal/arch/riscv/src/rv64/boot.rs` — Assembly entry (_start, trap setup)
- `hal/arch/riscv/src/common/uart_ns16550a.rs` — Serial UART
- `hal/arch/riscv/src/common/sbi.rs` — SBI calls (shutdown, time)
- `hal/arch/riscv/src/common/timer.rs` — SBI timer (scheduling)

**ARM AArch64 — FULLY IMPLEMENTED** ✅ (Ring-3 smoke testing in QEMU)  
**x86_64 — FULLY IMPLEMENTED** ✅ (Ring-3 smoke testing in QEMU)  
**RV32, AArch32 — TRAIT STUBS** (trait impls only, no boot code)

### Multi-Architecture Strategy

Use `#[cfg(target_arch = "riscv64")]` to conditionally compile:

```rust
#[cfg(target_arch = "riscv64")]
mod riscv;

#[cfg(target_arch = "arm")]
mod arm;

pub use crate::riscv::*;  // Or arm::* depending on build
```

---

## VirtIO Device Integration

### MMIO Memory Mapping

**Problem**: Limine bootloader does not report MMIO ranges in its memory map, causing device registers to become inaccessible after kernel paging is activated.

**Solution**: Explicit identity-mapping in `kernel/src/memory/paging.rs::init_kernel_paging()`:

```rust
// QEMU virt machine MMIO layout (RV64)
// CLINT (Core Local INTerrupt)
map(VAddr(0x0200_0000), PAddr(0x0200_0000), 0x10000, READABLE | WRITABLE | VALID);

// PLIC (Platform Level Interrupt Controller)
map(VAddr(0x0C00_0000), PAddr(0x0C00_0000), 0x0400_0000, READABLE | WRITABLE | VALID);

// UART0 + VirtIO MMIO devices (slot 0–7)
map(VAddr(0x1000_0000), PAddr(0x1000_0000), 0x0001_0000, READABLE | WRITABLE | VALID);
```

All MMIO regions are identity-mapped (VA = PA) for simplicity and to preserve bootloader-assigned addresses.

### VirtIO IRQ Dispatch Pattern

VirtIO devices on QEMU `virt` machine use PLIC IRQs with slot-based numbering:

| Device | MMIO Slot | Base Address | IRQ |
|--------|-----------|--------------|-----|
| UART0  | —         | 0x1000_0000  | 10  |
| VirtIO Block | 0 | 0x1000_1000 | 1 |
| VirtIO Input | 1 | 0x1000_2000 | 2 |
| VirtIO Net | 2 | 0x1000_3000 | 3 |
| ... | i | 0x1000_(i+1)000 | i+1 |

**IRQ Dispatch**: `kernel/src/task/drivers/virtio_blk.rs::vi_handle_virtio_irq(irq: u32)`

```rust
pub fn vi_handle_virtio_irq(irq: u32) -> bool {
    match irq {
        1 => virtio_blk::block_device_irq(),     // VirtIO block (slot 0)
        2 => virtio_input::input_device_irq(),   // VirtIO input (slot 1)
        3 => virtio_net::net_device_irq(),       // VirtIO net (slot 2)
        _ => false,  // Unknown IRQ
    }
}
```

**Per-Device Handler Responsibilities** (Phase 05 established):
1. Drain the used ring to retrieve completed requests and process data
2. **Acknowledge the IRQ** via `ack_irq(irq)` to clear device `InterruptStatus` register
3. Re-arm the device by publishing empty buffers back to the available ring
4. Wake any blocked tasks waiting on device I/O

**Interrupt Flow (Correct Pattern)**:
```
Device generates interrupt
  ↓
PLIC sets bit in Pending register
  ↓
PLIC delivers IRQ to CPU
  ↓
Kernel trap handler calls vi_handle_virtio_irq(irq)
  ↓
Device handler:
  - Process available data/requests
  - Call ack_irq(irq) to clear InterruptStatus
  - Refill available ring
  ↓
PLIC acknowledges via plic_complete()
  ↓
Device can fire next interrupt (if new data arrives)
```

**Critical Fix (Phase 05)**: Input device was not calling `ack_irq()`, leaving `InterruptStatus` register set. PLIC would immediately re-fire the same interrupt after `plic_complete()`, creating an infinite interrupt storm. This caused kernel to hang on first keystroke. Fix: Added `pub static INPUT_DEVICE_IRQ` and `pub fn ack_irq()` to `kernel/src/task/drivers/virtio_input.rs`; expanded `vi_handle_virtio_irq()` to dispatch to input device handler.

### FAT16 Persistence & Graceful Shutdown (Phase E)

**Hardening** (safety fixes, no behavior change):
- `cells/services/vfs/src/block_stream.rs` — SeekFrom::Current now validates result ≥ 0 before u64 cast (prevents underflow→seek to arbitrary LBA)
- `kernel/src/task/syscall.rs` — BlkRead/BlkWrite now reject sectors ≥ CELL_TABLE_BASE_LBA (82,000) to prevent cell from corrupting kernel bootstrap table

**Clean Shutdown Path**:
- Syscall 502 (raw, no `ViSyscall` enum entry) — kernel SBI SRST handler calls OpenSBI to power off
- `cells/apps/shell/src/cmd_sys.rs` — `shutdown` built-in command triggers graceful QEMU exit
- Test harness `wait_for_natural_exit()` allows disk image to flush before reboot

**Integration Test** (`vfs_fat16_reboot_persistence`):
- Writes marker to FAT16 `/data/`, issues shutdown, waits for QEMU clean exit
- Reboots against same disk image, reads marker back to prove write durability across power cycle
- **Critical bug fixed during this phase**: `shell.rs` had pre-parser echo handler that split by whitespace, completely bypassing redirect parser. Removed handler; echo now correctly goes through parser and supports OP_WRITE redirects.

---

## Public API (Kernel-Cell Boundary)

Located in `libs/api/`, these traits define the stable ABI:

### Filesystem (`ViFileSystem`, `ViFile`)
```rust
pub trait ViFileSystem {
    async fn open(&self, path: &str, flags: u32) -> ViResult<Box<dyn ViFile>>;
    async fn read_dir(&self, path: &str) -> ViResult<Vec<DirEntry>>;
}

pub trait ViFile {
    async fn read(&mut self, buf: Box<[u8]>) -> ViResult<Box<[u8]>>;
    async fn write(&mut self, data: &[u8]) -> ViResult<usize>;
    async fn seek(&mut self, pos: u64) -> ViResult<u64>;
}

// IPC Opcodes (Phase F: FAT16 Hardening)
// OP_WRITE (0x04): [opcode][path_len:u8][content_len:u16 LE][path][content]
//   - Effective message cap: min(512, 4 + path_len + content_len) bytes
//   - /data/* → FAT16, /tmp/* → RamFS
// OP_UNLINK (0x07): [opcode][path_len:u8][path]
//   - /data/* → FAT16, /tmp/* → RamFS (nested paths supported)
// OP_MKDIR (0x05): [opcode][path_len:u8][path]
//   - /data/* → FAT16 mkdir -p, /tmp/* → RamFS (nested paths supported)
```

### Block Devices (`ViBlockDevice`)
```rust
pub trait ViBlockDevice {
    async fn read(&self, sector: u64, count: u32) -> ViResult<Box<[u8]>>;
    async fn write(&self, sector: u64, data: &[u8]) -> ViResult<u32>;
}
```

### Networking (`ViTcpStack`, `ViTcpStream`, Typed IPC, TLS)
```rust
pub trait ViTcpStack {
    async fn listen(&self, addr: &str, port: u16) -> ViResult<Box<dyn ViTcpListener>>;
    async fn connect(&self, addr: &str, port: u16) -> ViResult<Box<dyn ViTcpStream>>;
}

// Primary IPC Format (Phase 27 — Protocol Hardening)
// Net service now uses typed postcard IPC as primary wire format:
// - NetRequest enum: CreateSocket, Connect, Bind, Send, Recv, Close, Listen, Accept, TlsConnect, TlsSend, TlsRecv, GetSocketState, etc. (15 variants)
// - NetResponse enum: SocketCreated, Connected, Bound, DataSent, DataReceived, SocketClosed, etc.
// - All variants type-checked at kernel dispatch; prevents serialization bugs and type confusion

// TLS 1.3 Client (Phase TLS-01) — typed + raw-opcode fallback
// Typed path (primary):
//   - NetRequest::TlsConnect { host, port, hostname } → NetResponse::TlsConnected { cap_id }
//   - NetRequest::TlsSend { cap_id, data } → NetResponse::TlsDataSent { bytes_written }
//   - NetRequest::TlsRecv { cap_id, max_len } → NetResponse::TlsDataReceived { data }
//
// Raw fallback (legacy, for backward compatibility with ostd::tls helpers):
//   - TLS_CONNECT (0x30): [addr:4 LE][port:2 LE][hostname:*] → [cap_id:8 LE]
//   - TLS_SEND (0x31): [data:*] → [bytes_written:4 LE]
//   - TLS_RECV (0x32): [max_len:4 LE] → [decrypted_data:*]
```

### Drivers (`ViDriver`)
```rust
pub trait ViDriver {
    fn name(&self) -> &str;
    fn probe(&mut self) -> ViResult<()>;
    fn capabilities(&self) -> u32;
}
```

### Runtime (`ViVmRuntime`)
```rust
pub trait ViVmRuntime {
    fn load(&mut self, bytecode: &[u8]) -> ViResult<()>;
    fn execute(&mut self, function: &str, args: &[Value]) -> ViResult<Value>;
}
```

---

## Cells (User-Space Software)

### What is a Cell?

A **Cell** is an isolated execution context (like a process) but:
- Shares kernel's address space (no context-switch overhead)
- Cannot use `unsafe` code (Rust enforces this)
- Communicates via syscalls (IPC, filesystem, logging)
- Has its own task control block, page table, and message queue

### Cellos App SDK (L1 Platform Layer)

**Purpose**: Eliminate boilerplate and unlock real native applications without kernel expertise.

**Components** (`libs/ostd/`):
- **`CellRuntime` builder**: Unified app initialization — handles manifest generation, permission sets, lifecycle
- **`app_entry!` / `service_entry!` macros**: Declarative entry points (10–30 lines replaces 200+ lines of manual boilerplate)
- **Typed client facades**:
  - `VfsClient` — read_file, write_file, append_file, stat, list_dir, mkdir, unlink
  - `NetClient` — tcp_connect, tcp_send, tcp_recv, tcp_close, dns_lookup, local_ip
  - `InputClient` — request_focus, get_focus, clear_focus
- **Lifecycle support**: `ShutdownReason` enum, `ShutdownWith` event, `arm_heartbeat()`, `run_with_lifecycle()` for graceful shutdown
- **Lazy service accessors**: `app.vfs()`, `app.net()` resolve on first use

**Reference app** (`cells/apps/hello-cell/`):
```rust
use api::{app_entry, CellRuntime};

app_entry!(handler = run);

async fn run() {
    println!("Hello from Cellos App SDK!");
}
```

**Impact**: Apps no longer need to understand manifests, syscall allowlists, or raw IPC — all abstracted by the SDK. Foundation for L2 middleware (HTTP servers, databases, pub-sub), unblocking G2 real application development.

### Cell Types

**Tools**: System utilities & CLI applications
```
cells/tools/shell/     — Interactive REPL (parser, executor, aliases, jobs, history)
cells/tools/init/      — Bootstrap (spawns vfs, config, input, net, compositor, shell, robot-demo; games/demos run on-demand from shell)
cells/tools/sys-tools/ — Standalone binaries: ls, cat, echo, ps, kill (0x2A000000 VA base)
cells/tools/net-tools/ — Network utilities: ping, curl, wget, nc, httpd, mqtt (0x26000000 VA base)
cells/tools/wasm/      — WASM interpreter cell (feature-gated)
```

**Applications**: User-facing applications
```
cells/apps/robot-dashboard/ — Reference G1 HMI dashboard (ViUI v2, 800×480, 0x0D000000 VA)
```

**Demos**: Hardware/feature demonstrations and graphical showcases
```
cells/demos/hello/           — Minimal test app
cells/demos/hello-cell/      — SDK reference (17-line zero-boilerplate app)
cells/demos/periph-demo/     — GPIO pin blink demo (QEMU ARM virt)
cells/demos/sensor-demo/     — I2C SHT3x temperature sensor (0x2E000000 VA)
cells/demos/spi-demo/        — SPI peripheral test (0x30000000 VA)
cells/demos/pwm-demo/        — PWM servo control
cells/demos/adc-demo/        — ADC analog input
cells/demos/can-demo/        — CAN bus messaging
cells/demos/robot-demo/      — End-to-end sensor→compute→actuator (GPIO ownership cycling, MQTT)
cells/demos/sdk-demo/        — Cellos App SDK patterns
cells/demos/https-demo/      — TLS 1.3 HTTPS client to example.com
cells/demos/viui-demo/       — ViUI v2 DSL → Rust codegen pipeline (Counter.vi)
cells/demos/audio-demo/      — VirtIO sound test tone (A4-C#5-E5 arpeggio, S16LE/2ch/44100)
cells/demos/doom/            — doomgeneric DOOM port (1024×768, 16MB quota, 0x42000000 VA); run: `doom`
cells/demos/tetris/          — Tetris in Rust-native Cell (ViUI)
cells/demos/tetris-c/        — Tetris via C platform hooks (demonstrates Tier 1b C pathway)
cells/demos/tetris-lua/      — Tetris scripted in Lua (demonstrates Tier 1b Lua pathway)
```

**Drivers**: Hardware device drivers
```
cells/drivers/disk/      — VirtIO block passthrough (✅ working)
cells/drivers/gpu/       — VirtIO GPU (opt-in framebuffer)
cells/drivers/input/     — VirtIO input passthrough (deprecated; kernel poll used)
cells/drivers/net/       — VirtIO NIC wrapper (deprecated; kernel poll used)
cells/drivers/gpio/      — PL061 GPIO driver (ARM64 QEMU virt)
cells/drivers/gpio-sifive/ — SiFive GPIO extension
cells/drivers/serial/    — PL011 UART driver (ARM64)
cells/drivers/i2c-gpio/  — BitBangI2c<G> generic over ViGpio
cells/drivers/spi-gpio/  — BitBangSpi<G> generic over ViGpio
cells/drivers/pwm-gpio/  — BitBangPwm<G> generic over ViGpio
cells/drivers/adc-sim/   — Simulated ADC (no MMIO)
cells/drivers/can-loopback/ — Loopback CAN (no MMIO)
cells/drivers/wasm/      — WASM runtime wrapper
```

**Services**: System services with long-lived state
```
cells/services/vfs/       — RamFS + FAT32 + littlefs + BootFS (✅ MountTable dispatch complete)
cells/services/config/    — Key-value store (✅ ViStateTransfer impl)
cells/services/compositor/  — Software blending + z-order + Grant surfaces
cells/services/input/     — Input event routing + focus system
cells/services/net/       — smoltcp TCP/IP + DHCP + TLS 1.3 (✅ typed postcard IPC)
cells/services/hypervisor/ — ARM64 EL2 VMM (Alpine Linux) (✅ minimal VMM)
cells/services/silo/      — Security Silo (Hardware key isolation, Stage-2 fence) (✅ complete)
cells/services/httpd/     — HTTP web server (shell builtin)
cells/services/power/     — Power management (stub)
```

**Runtimes**: VMs/interpreters for scripting
```
cells/runtimes/lua/       — Lua 5.4 via FFI (✅ REPL verified)
```

**Tests**: Integration & stress test cells
```
cells/tests/bench/           — RT + SMP latency benchmark (3 scenarios)
cells/tests/vfs-test/        — VFS service test suite (8 scenarios)
cells/tests/srv-test/        — Spawn + state transfer tests
cells/tests/hypervisor-test/ — Tier 3b VM lifecycle tests
cells/tests/gpio-test-rv/    — RISC-V GPIO integration
cells/tests/periph-test/     — Peripheral driver unit tests
cells/tests/posix-shim-test/ — POSIX stdio/math/setjmp tests
cells/tests/c-math-smoke/    — C runtime verification (12 scenarios, 3 arches)
cells/tests/mlibc-smoke/     — mlibc Tier B integration
cells/tests/input-test/      — Input service focus & event tests
cells/tests/silo-test/       — Security Silo (6 end-to-end test cases)
cells/tests/test-isolation/  — Cell fault isolation tests
```

**Guests**: Hypervisor guests (Tier 3b)
```
cells/guests/silo-guest/  — aarch64-unknown-none bare-metal (p256 ECDSA signing, secure enclave)
```

**UI Library** (`libs/viui/`): no_std UI toolkit for GUI app Cells
```
libs/viui/             — ViUI toolkit (no_std + alloc, MIT)
  v1 (done):           Elm model, FramebufferCanvas, GlyphAtlas — foundation
  v2 (G2 planned):     Reactive Signal Tree + Dual-Layer DSL (see below)
```

---

## ViUI Architecture (G2 Target)

ViUI v2 targets the constraints of Cellos's no_std Cell environment while matching the ergonomics of modern native UI toolkits.

### Dual-Layer Design

```
┌────────────────────────────────────────────────────────┐
│  Layer 1 — .vi DSL  (Slint-compatible syntax)          │
│                                                        │
│  component Counter {                                   │
│      in-out property <int> count: 0;                   │
│      VerticalLayout {                                  │
│          Text { text: "Count: \{count}"; }             │
│          Button { text: "+1"; clicked => {count+=1;} } │
│      }                                                 │
│  }                                                     │
│                                                        │
│  vi-compiler (build.rs) → generates Layer 2 Rust code  │
│  Hot-reload: watcher daemon, no recompile needed       │
└────────────────────────────────────────────────────────┘
                         ↓ compiles to
┌────────────────────────────────────────────────────────┐
│  Layer 2 — Rust Signal API  (also direct public API)   │
│                                                        │
│  #[vi_component]                                       │
│  struct Counter { count: Signal<i32> }                 │
│                                                        │
│  impl ViComponent for Counter {                        │
│      fn view(&self) -> impl ViNode {                   │
│          vstack!(                                      │
│              label!(text: self.count                   │
│                  .map(|n| format!("Count: {n}"))),     │
│              button!(text: "Increment",                │
│                  on_click: || self.count               │
│                      .update(|n| n+1)),                │
│          )                                             │
│      }                                                 │
│  }                                                     │
└────────────────────────────────────────────────────────┘
```

**Key properties**:
- Layer 1 uses Slint expression language → zero migration cost from Slint
- Layer 2 uses Rust expressions → familiar to Rust devs, no DSL required
- Signal<T> reactive engine: only affected widgets repaint → no full-screen repaints
- ViRenderer trait: FramebufferCanvas (CPU, no GPU needed) or GPU backend (G2+)
- no_std + alloc throughout; no std dependency in runtime crates

### Reactive Update Model

```
Signal<count>.set(42)
    ↓
Notify subscriber widgets (only label in this example)
    ↓
Mark label's dirty_rect
    ↓
Repaint only label region (~80×16 px)
    ↓  
surf.damage_rect(dirty)    ← NOT damage_all()
```

Contrast with ViUI v1 (Elm): every button click → rebuild all 20 widgets → layout all → repaint 307,200 px.

### Crate Layout

```
tools/vi-compiler/     (std, build tool)     — .vi parser, Slint expr evaluator, codegen
tools/viui-build/      (std, build-dep) ✅   — build.rs integration wrapper (P05 complete)
libs/viui-macros/      (proc_macro) ✅       — vi_design!{} for inline prototype use (P06 complete)
libs/viui-core/        (no_std + alloc)      — Signal<T>, LayoutNode, DirtyRect, ViRenderer trait
libs/viui-widgets/     (no_std + alloc)      — typed widget structs (Layer 2 API)
libs/viui/             (no_std, umbrella) ✅ — re-exports all above + viui_macros (P06 complete)
```

**P05 Build Integration** (2026-06-08): `tools/viui-build/` wraps vi-compiler; cells use `build.rs` → `viui_build::compile(glob)` → `include!()` generated Rust. Demo Cell (`cells/apps/viui-demo/`) validated end-to-end. Workspace `exclude` separates compiler from kernel/cells for independent versioning.

**P06 Proc Macro** (2026-06-08): `libs/viui-macros/` ships with `vi_design!` macro for inline component prototyping. `libs/viui` re-exports both paths (build.rs + macro); users import once, use both. Codegen redesigned to wrap each component in `mod __vi_generated_<Name>` to prevent symbol collisions.

Design brief: [.agents/brainstorms/260608-viui-nextgen-architecture.md](.agents/brainstorms/260608-viui-nextgen-architecture.md)

---

### Cell Lifecycle

```
1. Boot kernel
   ↓
2. Kernel spawns "init" Cell from embedded binary
   ↓
3. Init spawns "config" service (KV store)
   ↓
4. Init spawns "vfs" service (filesystem server)
   ↓
5. Init spawns "shell" application (interactive REPL)
   ↓
6. User types commands → shell sends IPC to vfs/config
   ↓
7. Shell displays output from services
   ↓
8. Ctrl+A X to shutdown
```

---

## Boot Sequence (Visual)

```
┌─────────────────────────────────────────────────┐
│ Bootloader (Limine or OpenSBI)                  │
│ Sets up: memory, DTB, argc/argv                 │
└──────────────┬──────────────────────────────────┘
               ↓
┌─────────────────────────────────────────────────┐
│ kernel/src/boot.rs: kmain(hartid, dtb)          │
│ 1. Initialize UART for logging                  │
│ 2. Parse bootloader info (memory map, DTB)      │
│ 3. Initialize HAL (traps, interrupt handler)    │
└──────────────┬──────────────────────────────────┘
               ↓
┌─────────────────────────────────────────────────┐
│ kernel/src/main.rs: _km_start()                 │
│ 4. Frame allocator (bitmap)                     │
│ 5. Virtual memory (SV39 paging)                 │
│ 6. Heap allocator (64 MB)                       │
│ 7. PLIC (interrupt controller)                  │
└──────────────┬──────────────────────────────────┘
               ↓
┌─────────────────────────────────────────────────┐
│ kernel/src/task.rs: init_scheduler()            │
│ 8. Task allocator (TCB pool)                    │
│ 9. Load "init" Cell from embedded FAT32         │
│ 10. Enter scheduler loop                        │
└──────────────┬──────────────────────────────────┘
               ↓
┌─────────────────────────────────────────────────┐
│ cells/apps/init/src/main.rs: main()             │
│ 11. Spawn "config" service via syscall::spawn() │
│ 12. Spawn "vfs" service                         │
│ 13. Spawn "shell" application                   │
│ 14. Idle (let scheduler handle)                 │
└──────────────┬──────────────────────────────────┘
               ↓
┌─────────────────────────────────────────────────┐
│ cells/apps/shell/src/main.rs: main()            │
│ 15. Print prompt: "Cellosh> "                     │
│ 16. Read user input (async)                     │
│ 17. Parse command (echo, cat, ls, etc.)         │
│ 18. Send IPC to vfs/config services             │
│ 19. Display response                            │
│ 20. Loop to step 15                             │
└─────────────────────────────────────────────────┘
```

---

## Memory Layout (SV39 RV64)

```
Virtual Address Space (64-bit, SV39 = 39-bit VA)
┌───────────────────────────────────┐
│  User Space (< 0x8000_0000)       │  Per-task, isolated via page table
│  - Stack (top, grows down)        │
│  - Heap (dynamic, grows up)       │
│  - Code/Data (ELF loaded here)    │
└─────────────────────────────────────┘  0x7fff_ffff

┌───────────────────────────────────┐
│  Guard Hole (unmapped)            │  0x8020_0000 - 0x7fff_ffff
│  Prevents user/kernel overflow    │
└───────────────────────────────────┘  0x8020_0000

┌───────────────────────────────────┐
│  Kernel Space (≥ 0x8020_0000)     │  Identity-mapped, shared
│  - Code: kernel binary            │
│  - Data: statics, globals         │
│  - Heap: kernel allocator         │
│  - Page tables (per-task)         │
│  - Task pool (TCBs)               │
└───────────────────────────────────┘  0xffff_ffff_ffff_ffff

Physical RAM: 0x8000_0000–0x8800_0000 (default: 128 MB in QEMU)
```

---

## IPC & Message Passing

### Send Message (Async)

```
┌────────────────────────────────────┐
│ Cell A (shell)                     │
│ syscall::send(vfs_id, msg, grant) │
│ (doesn't block, returns immediately)
└────────────────────┬───────────────┘
                     ↓
            ┌─────────────────┐
            │ Kernel          │
            │ - Validates msg │
            │ - Queues in VFS │
            │ - Wakes VFS     │
            └────────┬────────┘
                     ↓
            ┌─────────────────┐
            │ Cell B (vfs)    │
            │ woken by kernel │
            │ syscall::recv() │
            └─────────────────┘
```

### Call & Reply (RPC)

```
┌────────────────────────────────────┐
│ Cell A (shell)                     │
│ syscall::call(vfs_id, req, cap)   │
│ BLOCKS, waiting for reply          │
└────────────────────┬───────────────┘
                     ↓
            ┌─────────────────┐
            │ Kernel          │
            │ - Queues msg    │
            │ - Blocks Cell A │
            └────────┬────────┘
                     ↓
            ┌──────────────────────┐
            │ Cell B (vfs)         │
            │ syscall::recv()      │
            │ → gets request       │
            │ process...           │
            │ syscall::reply(A, rsp)
            └────────┬─────────────┘
                     ↓
            ┌─────────────────┐
            │ Kernel          │
            │ - Unblocks A    │
            │ - Delivers rsp  │
            └────────┬────────┘
                     ↓
            ┌──────────────────────┐
            │ Cell A resumes       │
            │ receives reply       │
            │ continues...         │
            └──────────────────────┘
```

---

## Current Status (2026-06-05)

### ✅ Implemented (Phases 01, 02, 05, 10, 14, 15, 16, 18, 20, C–H, A–E, X-1–X-3, Peripheral Driver Track v1, Robot Demo)
- **RV64, AArch64, x86_64** HAL with paging (SV39/4K/4K respectively)
- **Nano kernel** (~8,700 LOC) with round-robin scheduler
- **48 syscall variants** (IPC, memory, task, FS, GPU, network, state) + **Block I/O capability gate**
- **Block I/O syscalls** (raw 500/501/503 for FAT16 persistence, gated to VFS task 3)
- Frame allocator (bitmap) and virtual memory
- ELF loader with PIE relocation support
- **VFS service** (RamFS read/write, FAT32 write/read/delete via block device, zero-copy grants)
  - **10 IPC opcodes** (0x01–0x0A): OP_GET_FILE, OP_LIST_DIR, OP_STAT, OP_WRITE, OP_MKDIR, OP_RMDIR, OP_UNLINK, **OP_READ, OP_RMDIR_RECURSIVE, OP_APPEND**
  - **Zero-copy grants** (syscalls 208–212): GrantAlloc, GrantShare, GrantSlice, GrantFree, BlkReadAsync
  - **4-byte OP_WRITE header** (u16 content length, up to 65KB writes per message)
  - **OP_READ (0x08)** — read file bytes (up to 480, path → bytes)
  - **OP_APPEND (0x0A)** — seek-to-end append write
  - **OP_RMDIR_RECURSIVE (0x09)** — recursive directory delete (restricted to /data/ path prefix)
  - **OP_UNLINK** for /data/ flat files and nested paths
  - **/data/ subdirectories** with mkdir -p semantics and full path traversal
  - **OP_MKDIR** for /data/ nested directory creation
- **FAT32 filesystem** (LBA 0–524,287 on VirtIO disk, 540K sectors, /data/* paths persistent with subdir support)
- **Config service** (KV store with ViStateTransfer)
- **Interactive shell** (parser+executor) with:
  - Pipes, redirection (>, >>), background jobs (&), history, aliases
  - for/in/do/done, while/do/done, if/then/else/fi loops
  - case/esac conditional, shell functions (name() {}), **command substitution $(cmd)**
  - **Function arguments** ($1, $2, ..., $9)
  - **read built-in** for input
  - 45+ built-in commands
- **Lua 5.4** runtime (multi-line REPL, VFS I/O FFI, ViStateTransfer, network bindings) — verified
- **MicroPython 1.24.1** runtime (REPL, 256KB heap, vnet+DNS+UDP modules) — verified
- **Keyboard input** (VirtIO, multi-key support, no deadlock)
- **Network** (smoltcp TCP/UDP/DNS, DHCP verified, full data-path TCP client+server)
  - **TCP client**: SOCKET_TCP, CONNECT, SEND, RECV, CLOSE
  - **TCP server**: LISTEN (0x17), ACCEPT (0x18) opcodes
  - **UDP**: SOCKET_UDP, SENDTO (0x21), RECVFROM (0x22), BIND
  - **DNS resolver**: static table → IPv4 literal → UDP A-record query
  - **net-tools binaries** (6 total): ping, curl (HTTP/1.0), wget, nc (multi-conn relay), httpd, mqtt (skeleton)
- **GPU framebuffer** (opt-in, basic compositor)
- **HotSwap orchestrator** (5-step live Cell replacement, kernel + shell + config + vfs + robot-demo verified)
- **Peripheral Driver Track v1** (GPIO/UART HAL traits + driver Cells + safe MMIO + Resource Registry)
  - `cells/drivers/driver-gpio/` — PL061 GPIO implementation (QEMU ARM virt)
  - `cells/drivers/driver-serial/` — PL011 UART extension
  - `ostd::mmio::MmioRegion` — safe memory-mapped I/O (forbids unsafe in Cells)
  - Manifest-based capability gating via `declare_manifest!(gpio=true, uart=true)` (Phase 30)
- **Robot Demo (`cells/apps/robot-demo/`)** — Reference G1 closed-loop application
  - Sensor read (GPIO input) → control compute → actuator write (GPIO output)
  - MQTT 3.1.1 client: TcpConnect → handshake → publish telemetry → close
  - 7-cell boot sequence: vfs, config, input, net, compositor, shell, robot-demo
  - Graceful fallback to simulation when GPIO unavailable
  - Policy: Temporary (run once, no restart)
- **Workspace consolidated** with 0 cargo warnings
- **CI/CD pipeline** with architecture validation (10/10 score)
- **VirtIO VA→PA mapping fix** (Phase X-1) — resolves multi-sector write issues

### 🚧 In Progress / Partial
- **MQTT binary** (skeleton added; implementation deferred)
- **KASLR** (not implemented)

### ⏳ Planned (Later phases)
- **ViUI v2 — Reactive Signal Tree + Dual-Layer DSL** `[G2]`:
  - Layer 1: `.vi` files with 99% Slint-compatible syntax; vi-compiler (build.rs) generates Layer 2 Rust code; hot-reload via watcher daemon
  - Layer 2: typed `Signal<T>`-based Rust API — what the compiler generates, also a direct public API for Rust devs; no `Box<dyn>` per update, near-zero allocation per state change
  - Reactive engine: `Signal<T>` notifies only affected widgets → repaint only dirty rects
  - `ViRenderer` trait: software rasterizer (CPU default, G1) swappable with GPU backend (G2+)
  - Design brief: [.agents/brainstorms/260608-viui-nextgen-architecture.md](.agents/brainstorms/260608-viui-nextgen-architecture.md)
- ARM64 full kernel bring-up (pending, needed for real GPIO on aarch64 QEMU ARM virt)
- Peripheral Driver extensions: I2C, SPI, CAN, PWM, ADC (G1 ext / G2)
- Real SBC validation (RPi 4 / VisionFive2 / Radxa ROCK 5)
- Reliability track: stack guard pages, deadline/watchdog enforcement, supervisor-based
  cell restart, reboot-on-kernel-panic — see [specs/12-reliability.md](specs/12-reliability.md)
- Tier 3 hypervisor cell (Stage-2 paging) — hardware isolation for untrusted/legacy code
- Ed25519 signing + secure-boot loader gate (enforces the Tier 1 "signed cells only" model)
- Audit logging
- Additional architecture ports

> ⚠️ **Per-Cell SATP isolation at Tier 1 is explicitly NOT pursued** (decided 2026-06-05).
> Hardware isolation belongs to Tier 3 (per-VM Stage-2 paging), not per-cell page tables.
> See *Key Design Decisions* below and [specs/05-application.md](specs/05-application.md).

---

## Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| Single Address Space | Reduce context-switch overhead, simplify memory management |
| Language-Based Isolation | Rust's type system enforces isolation better than hardware |
| **No per-Cell SATP (Tier 1)** | Per-cell page tables would break Tier 1 zero-copy IPC and add `sfence.vma` cost on every switch (ASID broken on most RV silicon). Untrusted code is confined to Tier 2 (WASM) / Tier 3 (Stage-2). Decided 2026-06-05. |
| Tiered isolation (1/2/3) | Trusted signed-native (LBI) · WASM software sandbox · hypervisor hardware silo — isolation strength scales with untrust, hardware MMU cost paid only at Tier 3 |
| Round-Robin Scheduler | Simple, fair, predictable for embedded real-time systems |
| Capability-Based Access | Fine-grained control, no global permissions |
| Owned Buffers in Async | Deterministic cleanup in SAS (no process teardown) |
| Nano Kernel (~8,700 LOC) | Keep TCB, minimize trusted code, move features to Cells |
| Trait-Based HAL | Multi-architecture support without code duplication |
| No mod.rs | Clearer module boundaries, IDE-friendly |

---

## Architecture Gap Summary

Areas where the current implementation diverges from the specification or modern OS best practices. Tracked for resolution in Phases 24–32.

| Gap | Impact | Target Phase |
|-----|--------|-------------|
| IPC is syscall-based, not direct vtable call | 10–100× latency vs. spec | Phase 27 |
| Round-robin scheduler, no priority levels | RT tasks can starve | Phase 25 |
| No KASLR | Kernel address predictable | Phase 24 |
| No per-cell memory quota enforcement | Single cell can OOM system | Phase 26 |
| Spectre v1/v2 unmitigated in SAS | Critical for untrusted code | Phase 28+ (Tier 3 VM) |
| Tier 2 WASM runtime absent | No safe third-party code execution | Phase 29 |
| TLSF allocator not implemented | RT allocation guarantee broken | Phase 25 |
| No audit ring buffer | Forensics impossible | Phase 26 |
| Performance baseline unmeasured | Can't validate PDR targets | Phase 24 (immediate) |

---

## See Also

- **CLAUDE.md** — 8 Coding Laws & quick reference
- **api-reference.md** — Full trait & syscall reference
- **patterns.md** — Common code patterns
- **codebase-summary.md** — File structure & LOC counts
- **code-standards.md** — Code style & naming
- **Specs**: `docs/specs/0X-*.md` — Detailed subsystem specifications
