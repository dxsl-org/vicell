# Cellos — Innovation Proposals

**Version**: 1.0  
**Last Updated**: 2026-06-08  
**Tác giả**: Phân tích từ competitive landscape + kiến trúc Cellos hiện tại  
**Trạng thái**: Đề xuất — chưa phải roadmap chính thức

> Mỗi đề xuất đều bắt đầu bằng câu hỏi: **"Tại sao Linux/Windows/FreeRTOS không làm điều này?"**  
> Nếu câu trả lời là "họ có thể làm nhưng chưa làm" → ý tưởng bình thường.  
> Nếu câu trả lời là "họ không thể làm vì phải tương thích ngược" → đây là cơ hội thực sự của Cellos.

---

## Nền tảng: Cellos có 4 "siêu năng lực" độc nhất

Trước khi đề xuất, cần hiểu tại sao những thứ sau đây khả thi với Cellos mà không khả thi với hệ thống khác:

| Siêu năng lực | Ý nghĩa | Không thể có ở Linux/Windows |
|---|---|---|
| **Cell boundary knowledge** | Kernel biết chính xác vùng nhớ nào thuộc Cell nào (Metadata Registry) | Linux biết process boundary nhưng không biết library/component boundary bên trong process |
| **Typed inter-cell call** | Mọi cross-cell call là typed Rust trait call qua vtable | Linux syscall là untyped bytes; IPC giữa process là untyped streams |
| **Live Cell lifecycle** | Kernel có thể pause, serialize state, reload, re-link bất kỳ Cell nào đang chạy | Không thể làm với process mà không dừng toàn hệ thống |
| **No backward compat** | Không có 50 năm POSIX/Win32 API cần hỗ trợ | Linux phải compile kernel với 30-year-old syscall, Windows với NT 3.1 ABI |

---

## Đề xuất 1: Observer Cell — Profiling production zero-overhead

### Vấn đề
Profiling production system là bài toán khó của mọi OS:
- **Sampling profiler** (perf, dtrace): 1-5% overhead kể cả khi bạn không cần data
- **Instrumented build**: recompile toàn bộ, code path thay đổi → profiling ảnh hưởng kết quả
- **eBPF**: kernel-level, phức tạp, không type-safe, overhead khi probe active

### Tại sao Linux không thể làm tốt hơn
Không có API typing cho inter-process call. eBPF probe phải hook vào kernel function hoặc userspace memory address — fragile, version-specific, nguy hiểm khi sai địa chỉ.

### Cellos có thể làm gì
**Mọi inter-cell call đều đi qua vtable entry trong kernel-managed dispatch table.**

Observer Cell là Cell đặc biệt có thể:
1. Đăng ký với kernel: "Tôi muốn intercept mọi call tới `ViFileSystem::read` của Cell X"
2. Kernel swap vtable entry: `cell_x.vtable[read] = observer_cell.intercept_read`
3. Observer ghi timestamp, call the real function, ghi elapsed time
4. Khi Observer unlinked: kernel restore vtable về original function pointer

```rust
// Observer Cell contract — không cần thay đổi Cell bị observe
pub trait ViObserver {
    fn on_call(&self, target_cell: CellId, method: &str, args_hash: u64) -> ObserveToken;
    fn on_return(&self, token: ObserveToken, result_ok: bool);
}
```

**Overhead khi không có Observer**: tuyệt đối zero — vtable entry trỏ thẳng vào function gốc.  
**Overhead khi Observer active**: 1 indirect call thêm (~5ns) + Observer logic.

### Ứng dụng
- Production performance analysis không cần recompile, không cần downtime
- Circuit breaker: Observer Cell tự động unlink Cell nếu error rate vượt ngưỡng
- Distributed tracing tự động (xem Đề xuất 4)
- A/B testing performance: so sánh Cell v1 vs v2 trong production với exact same workload

### Feasibility
Cần thêm kernel API `sys_register_observer(target_cell, trait_id, method_id)`. Tương thích hoàn toàn với kiến trúc vtable hiện tại. Ưu tiên: **G2**.

---

## Đề xuất 2: Typed Zero-Copy Ring Channel

### Vấn đề
IPC hiện tại của Cellos (`sys_send`) là synchronous request-response. Tốt cho lệnh, tệ cho **data streaming liên tục** (sensor 1kHz, camera 30fps, audio 48kHz).

Với loại data này, mọi OS đều phải dùng shared memory + semaphore — untyped, error-prone, crash khi một bên chết.

### Tại sao Linux không thể làm tốt hơn
POSIX pipe là untyped bytes. Shared memory qua `mmap` là untyped void*. Không có gì ngăn producer và consumer dùng struct khác nhau → memory corruption. Không có gì xử lý khi producer Cell chết trong lúc consumer đang đọc.

### Cellos có thể làm gì
**`CellChannel<T>` — kernel-owned typed ring buffer:**

```rust
// Producer Cell
let chan = sys_create_channel::<SensorReading>(capacity: 64)?;
sys_publish_channel(chan, SERVICE_SENSOR)?;     // đặt tên vào registry

// Consumer Cell
let chan = sys_open_channel::<SensorReading>("sensor.imu")?;
while let Some(reading) = chan.recv_nonblocking() {
    // SensorReading ở đây là OWNED — không copy, kernel transfer ownership
}
```

**Đặc điểm:**
- Kernel sở hữu ring buffer trong Metadata Registry — buffer tồn tại kể cả khi producer/consumer Cell bị restart
- Khi producer Cell crash: kernel đánh dấu channel `Disconnected`, consumer nhận `Err(Disconnected)` thay vì invalid memory
- Khi consumer restart: reconnect vào channel, tiếp tục từ oldest unread frame
- Backpressure: `send()` trả `Err(Full)` — không bao giờ block, không bao giờ OOM
- Type-safe: schema hash của `T` được check tại `sys_open_channel` — Cell version mismatch = error rõ ràng

### Ứng dụng
- **Sensor fusion pipeline**: IMU Cell → Kalman Filter Cell → Motion Controller Cell — toàn bộ zero-copy, typed
- **Camera pipeline**: Camera Driver Cell → Inference Cell → Display Cell
- **Audio**: Mic Cell → DSP Cell → Speaker Cell (latency < 5ms)
- **Log streaming**: mọi Cell ghi vào `CellChannel<LogEntry>`, Log Aggregator Cell đọc

### Feasibility
Kernel cần thêm ring buffer allocator trong Metadata Registry. Phức tạp vừa phải. **Ưu tiên: G1/G2 boundary.**

---

## Đề xuất 3: Adapter Cell — Semantic Version Negotiation

### Vấn đề
Khi Cellos ecosystem phát triển, `ViFileSystem` sẽ có v1.x, v2.x, v3.x. Nếu App Cell cần v2.x và VFS Cell implement v3.x → không tương thích. Hiện tại: không có giải pháp.

### Tại sao Linux không thể làm
Dynamic library versioning (soname) trên Linux là manual: maintainer phải viết compat shim, ship `libfoo.so.2` và `libfoo.so.3` song song. Không có automation. ELF không có type information để auto-generate shim.

### Cellos có thể làm gì
Vì inter-cell API là Rust trait có đầy đủ type signature (bao gồm semver từ Cargo.toml), linker có thể **tự động generate Adapter Cell**:

```
[Cell Linker phát hiện mismatch]
App Cell cần:     ViFileSystem v2.3 (method: read(path: &str) -> Box<[u8]>)
VFS Cell expose:  ViFileSystem v3.0 (method: read(path: &Path, opts: ReadOpts) -> ViResult<Box<[u8]>>)

→ Linker auto-generates:
pub struct VfsAdapterV2ToV3;
impl ViFileSystem_v2 for VfsAdapterV2ToV3 {
    fn read(&self, path: &str) -> Box<[u8]> {
        self.inner.read(Path::new(path), ReadOpts::default()).unwrap_or_default()
    }
}
→ Load Adapter Cell giữa App và VFS
```

**Breaking change (major version)**: cần Adapter Cell viết tay do developer + ship vào Cell store. Linker từ chối auto-generate → bắt buộc human review.

### Ứng dụng
- OTA update VFS/Net/Driver Cell lên major version mà không cần update tất cả App Cell cùng lúc
- Gradual migration: 90% app dùng v2 adapter, 10% app đã migrate sang v3 native
- Vendor compatibility: third-party Cell cũ vẫn chạy khi Cellos platform update

### Feasibility
Phức tạp cao — cần type-level diff analysis trong linker. **Ưu tiên: G2 sau khi ecosystem đủ lớn.**

---

## Đề xuất 4: Kernel-Native Distributed Tracing

### Vấn đề
OpenTelemetry/Jaeger yêu cầu: (1) manual instrumentation từng function, (2) SDK import vào mỗi service, (3) side-car collector process, (4) network overhead gửi trace data. Kết quả: developer ngại dùng, trace coverage thấp.

### Tại sao Linux không thể tự động
Inter-process call trên Linux đi qua kernel (syscall) nhưng kernel không biết "context" của call — trace context là userspace concept. Không thể auto-propagate qua pipe/socket mà không modify mọi app.

### Cellos có thể làm gì
**Mọi inter-cell call đều là typed, kernel-visible.** Kernel có thể inject trace context tự động:

```
[Cell A] gọi [Cell B].process()
  → Kernel: nếu trace active, tạo span {id, parent_id, cell_a, method="process", t_start}
  → Kernel: inject span_id vào call context (hidden field trong dispatch struct)
  → Cell B nhận call; nếu gọi Cell C, kernel đọc span_id → tạo child span
  → Kernel: khi call return, ghi t_end → emit span vào trace ring buffer
```

Trace ring buffer là `CellChannel<TraceSpan>` (từ Đề xuất 2) — Trace Collector Cell đọc và forward.

**Developer không cần viết một dòng instrumentation code nào.**

Kết quả: waterfall trace đầy đủ của mọi request chảy qua hệ thống, tự động, từ lúc boot.

### Ứng dụng
- Debug production latency spike: "request này tại sao 200ms?" → xem toàn bộ Cell call chain
- SLA enforcement: Cell contract "method X < 10ms" → trace tự động alert khi vi phạm
- Security audit: phát hiện Cell gọi những Cell nó không được phép gọi

### Feasibility
Cần trace ring buffer (Đề xuất 2) và kernel dispatch modification. Overhead có thể tắt/bật per-Cell. **Ưu tiên: G2.**

---

## Đề xuất 5: Middleware Cell Injection — Không Cần Recompile Target

### Vấn đề
Thêm cross-cutting concerns (rate limiting, circuit breaker, TLS termination, auth check, caching) vào một service thường nghĩa là: modify source code của service đó, hoặc dùng service mesh với proxy sidecar (Envoy, Linkerd) — cồng kềnh, thêm network hop.

### Tại sao Linux không thể làm sạch hơn
`LD_PRELOAD` trick tương tự nhưng: chỉ hoạt động với C/POSIX ABI, không type-safe, không thể revoke khi app đang chạy, không kernel-managed.

### Cellos có thể làm gì
Extend vtable swap từ Đề xuất 1 thành **Middleware Chain**:

```
Trước:  [App Cell] ──vtable──→ [Net Cell.connect()]

Sau khi inject RateLimiter:
        [App Cell] ──vtable──→ [RateLimiter.connect()]
                                    │ nếu quota OK
                                    └──→ [Net Cell.connect()]

Sau khi inject CircuitBreaker + RateLimiter:
        [App Cell] ──vtable──→ [CircuitBreaker.connect()]
                                    │
                                    └──→ [RateLimiter.connect()]
                                              │
                                              └──→ [Net Cell.connect()]
```

Middleware Cell implement cùng trait với target (`ViTcpStack`), forward về real Cell. Kernel quản lý chain — add/remove at runtime, không reboot, không recompile.

```rust
// Ops:
sys_inject_middleware(target_cell: CellId, trait: TraitId, middleware: CellId, position: Before|After);
sys_remove_middleware(target_cell: CellId, trait: TraitId, middleware: CellId);
```

### Ứng dụng
- **Rate limiting** cho Net Cell: không cần modify net Cell code
- **TLS termination**: inject TLS Cell trước Net Cell cho mọi App Cell cần HTTPS
- **Auth middleware**: inject token validator trước VFS Cell cho production
- **Caching**: inject Cache Cell trước VFS Cell cho read-heavy workload
- **A/B testing**: inject Shadow Cell ghi copy requests sang canary Cell

### Feasibility
Cần kernel middleware chain management (linked list of vtable entries). Phức tạp vừa phải. **Ưu tiên: G2.**

---

## Đề xuất 6: Fine-Grained Network Capability Micro-Segmentation

### Vấn đề hiện tại
`NetworkCap` hiện tại là binary: có hoặc không. Nếu Cell có NetworkCap, nó có thể kết nối đến bất kỳ IP nào trên bất kỳ port nào.

Với robot deployed thực tế: camera AI Cell cần kết nối đến inference server nội bộ, nhưng không được phép gọi ra internet. Nếu Camera Cell bị compromise → có thể exfiltrate data. Không có cách enforce điều này hiện nay.

### Tại sao Linux không làm tốt
iptables/nftables enforce theo PID/UID — crude. Kubernetes NetworkPolicy là external policy engine, không trong kernel, có thể bypass. SELinux network policy phức tạp cấu hình, không developer-friendly.

### Cellos có thể làm gì
**Network capability là typed struct, not binary flag:**

```rust
// Trong Cell manifest (ELF section)
capabilities: [
    NetworkCap::Connect {
        allow: [IpNet::new("192.168.1.0/24"), IpNet::new("10.0.0.1/32")],
        ports: PortRange(443..=443),
    },
    NetworkCap::Listen { port: 8080 },
    // Không có NetworkCap::Connect cho internet → bị block
]
```

Net Cell enforce tại `connect()` time: check `caller_cell.network_cap.allows(dst_ip, dst_port)`. Fail-fast với typed error `Err(ViError::CapabilityDenied)`.

**Kết hợp với Middleware Injection (Đề xuất 5):** Inject a "NetworkPolicy" middleware toàn hệ thống không cần sửa từng Cell.

### Ứng dụng
- **Robot**: sensor Cell chỉ được talk to local inference Cell, không ra internet
- **Server**: inference Cell chỉ được talk to model store Cell và result Cell — blast radius của compromise bị giới hạn
- **Multi-tenant G2**: Cell của tenant A không thể reach Cell của tenant B dù cùng machine
- **Compliance**: enforce "no PII leaves this subnet" ở kernel level, không ở application level

### Feasibility
Extend manifest parser + Net Cell runtime check. Công việc nhỏ, giá trị lớn. **Ưu tiên: G1 (security extension).**

---

## Đề xuất 7: Cell Graph API — Live Topology & Impact Analysis

### Vấn đề
Với hệ thống 20+ Cells đang chạy, operator không có cách biết:
- Cell nào đang depend vào Cell nào?
- Nếu update Cell X, những Cell nào sẽ bị ảnh hưởng?
- Cell nào đang chiếm nhiều memory/CPU nhất?
- Cell nào đang trong trạng thái Poisoned/Zombie?

### Tại sao Linux không có
Linux không có concept "dependency graph giữa services" ở kernel level. systemd có dependency graph nhưng static, không live, không biết về runtime call patterns.

### Cellos có thể làm gì
Kernel đã duy trì Cell DAG (strong/weak refs, Metadata Registry). Chỉ cần expose:

```rust
// Syscall mới
pub fn sys_get_cell_graph() -> CellGraph {
    CellGraph {
        cells: Vec<CellInfo>,    // id, name, version, state, memory_used, cpu_ms
        edges: Vec<CellEdge>,    // (from, to, ref_type, call_freq_per_sec)
    }
}
```

Từ đây có thể build:
- **Impact analysis**: "nếu tôi update Cell X, highlight tất cả Cell phụ thuộc vào X (transitive)"
- **Anomaly detection**: Cell Y đột ngột tăng call frequency lên Cell Z → alert
- **Live visualization**: web UI render graph real-time (dot/d3.js)
- **Dependency security audit**: "Cell A có kết nối tới Cell B không?" — câu hỏi compliance

### Ứng dụng
- Robot fleet dashboard: operator nhìn topology toàn bộ robot trong một screen
- CI/CD: trước khi deploy Cell mới, auto-check impact graph → warn nếu nhiều Cell bị ảnh hưởng
- Debug: "tại sao latency tăng?" → nhìn graph thấy edge từ App → VFS có call_freq tăng 10x

### Feasibility
Kernel đã có internal Cell DAG. API chỉ cần serialize và expose. **Ưu tiên: G1/G2, low effort, high value.**

---

## Đề xuất 8: TensorChannel — Zero-Copy NPU Pipeline (G3 prep)

### Vấn đề
AI inference trên mọi platform hiện tại đều có copy overhead:
- **Linux**: userspace app → copy buffer → RKNN SDK → copy to NPU DRAM → inference → copy result back
- **CUDA**: `cudaMemcpy` host-to-device + device-to-host = 2 copies trên PCIe bus
- **Android NNAPI**: HAL abstraction với copy ở mỗi layer

Với model 50MB inference ở 30fps: **1.5GB/s copy overhead** chỉ để move data vào/ra NPU.

### Tại sao Linux không thể tránh
Memory mapping giữa CPU và NPU phụ thuộc hardware (UMA vs NUMA). Linux unified memory (CUDA Unified/DMA-BUF) tồn tại nhưng: không type-safe, không có lifetime tracking, race condition nếu CPU và NPU access cùng lúc.

### Cellos có thể làm gì
**`TensorBuffer` — kernel-managed dual-domain memory:**

```rust
// Vùng nhớ đặc biệt: physically contiguous, mappable cả CPU và NPU
let tensor = sys_alloc_tensor_buffer(
    shape: &[1, 3, 224, 224],
    dtype: TensorDtype::F32,
    domain: TensorDomain::CpuNpu,   // Pin tới DRAM tối ưu cho cả CPU và NPU
)?;

// Camera Cell viết vào tensor — zero copy
tensor.as_cpu_slice_mut().copy_from_slice(&frame_data);
sys_flush_tensor_to_npu(&tensor);   // cache flush nếu cần

// Inference Cell đọc — NPU access trực tiếp
npu_cell.infer(&tensor, &output_tensor)?;   // NPU DMA từ tensor buffer, không qua CPU
```

**Kernel tracking**: `TensorBuffer` trong Metadata Registry với trạng thái `CpuOwned | NpuAccess | Shared`. Rust borrow rules ở compile time + kernel state machine tại runtime → không bao giờ race condition.

**Kết hợp với `CellChannel<TensorBuffer>`** (Đề xuất 2): streaming inference pipeline hoàn chỉnh zero-copy từ camera driver đến output.

### Ứng dụng
- Real-time object detection ở 30fps với latency < 10ms (vs ~50ms trên Linux)
- Multi-model inference pipeline: model A output là model B input — cùng TensorBuffer, không copy
- Robot visual servoing (camera → inference → control) với latency thực sự bounded

### Feasibility
Cần hardware-specific memory allocation (RKNN, X390 APIs) + kernel TensorBuffer type. Phức tạp cao. **Ưu tiên: G3 — nhưng thiết kế API từ G2 để không phá vỡ sau.**

---

## Tổng kết: Lộ trình đề xuất

| # | Tên | Unique to Cellos | Effort | Value | Priority |
|---|---|---|---|---|---|
| 1 | Observer Cell (zero-overhead profiling) | ✅ vtable-based, không thể làm với process model | Medium | Rất cao | **G2 early** |
| 2 | Typed Zero-Copy Ring Channel | ✅ kernel-owned typed buffer, Cell crash-safe | Medium | Cao (robot + server) | **G1/G2** |
| 3 | Adapter Cell (semver negotiation) | ✅ chỉ khả thi với typed trait ABI | High | Cao (ecosystem) | G2 |
| 4 | Kernel-native Distributed Tracing | ✅ tự động từ vtable, zero instrumentation | Medium | Cao | G2 |
| 5 | Middleware Cell Injection | ✅ vtable swap, live, no recompile | Medium | Cao | G2 |
| 6 | Network Capability Micro-segmentation | ✅ typed NetworkCap, kernel-enforced | Low | Cao (security) | **G1 ext** |
| 7 | Cell Graph API (live topology) | ✅ kernel đã có DAG, chỉ cần expose | Low | Cao (ops) | **G1/G2 easy win** |
| 8 | TensorChannel (zero-copy NPU) | ✅ kernel-managed dual-domain memory | Very High | Killer feature | G3 |

### Ba quick wins nên làm trước
- **Đề xuất 6**: Extend NetworkCap thành typed struct — ít code, giá trị security rõ ràng, không cần feature mới
- **Đề xuất 7**: Expose Cell Graph API — kernel đã có data, chỉ cần serialization syscall
- **Đề xuất 2**: Typed Ring Channel — unblocks camera/sensor pipelines, cần thiết cho robot demo thực tế

### Một đề xuất "flagship" cho positioning
**Đề xuất 5 (Middleware Injection)** + **Đề xuất 4 (Auto Tracing)** kết hợp tạo ra story mạnh nhất để phân biệt Cellos với mọi OS khác:

> *"Trong Cellos, bạn có thể thêm rate limiting, distributed tracing, circuit breaker, và TLS termination vào bất kỳ service nào, không cần chạm vào source code của service đó, không cần reboot, không cần proxy sidecar."*

Đây là điều mà Kubernetes service mesh (Istio, Linkerd) cố làm ở infrastructure layer nhưng phải chấp nhận network overhead + operational complexity. Cellos có thể làm điều tương đương ở OS layer, trong cùng address space, với zero overhead khi không active.
