# ViOS Architecture: Hardware Layer
**Version**: 0.3 (Universal HAL & Multi-Arch Strategy)
**Status**: Definitive

---

## 1. Multi-Architecture Strategy (The "Trait" Contract)
ViOS không phụ thuộc vào một kiến trúc CPU cụ thể. Mọi tương tác phần cứng được trừu tượng hóa qua crate `hal-core`.

### Trait-Based Abstraction
| Trait | Vai trò | Đặc điểm đa kiến trúc |
| :--- | :--- | :--- |
| **`Arch`** | Quản lý Context, Paging | Tự thích nghi bit-width (usize) và chuẩn phân trang (Sv32/39/48). |
| **`Interrupt`** | Đăng ký và điều phối ngắt | Hỗ trợ PLIC (RISC-V), GIC (ARM) hoặc APIC (x86). |
| **`Timer`** | Quản lý thời gian thực | Cung cấp độ phân giải cao cho Scheduler. |

## 2. Platform HAL vs. Device Driver Cells
* **Platform HAL**: Được biên dịch **cùng** Nano Kernel. Chịu trách nhiệm khởi tạo CPU, RAM và các thành phần cốt lõi.
* **Driver Cells**: Được nạp động dưới dạng **Cells**. Chịu trách nhiệm cho các ngoại vi (NIC, GPU, cảm biến Robot).

### Chiến lược WASM Sandboxed Drivers (Cứu cánh Driver cũ)
Để lấp đầy khoảng trống driver, ViOS cho phép chạy driver C từ Linux trong sandbox:
1. **Cơ chế**: Biên dịch C Driver sang **WASM**.
2. **Cách ly**: Chạy trong `WasmDriverRuntime` Cell. Lỗi tràn bộ đệm (Buffer Overflow) của driver C chỉ phá hủy bộ nhớ trong WASM, không thể làm sụp đổ Kernel.
3. **Hiệu năng**: Chấp nhận mức 50-70% cho các thiết bị không đòi hỏi tốc độ cao (HID, Audio, I2C).

## 3. Interrupt Model: "Async Waker Dispatch"
ViOS sử dụng mô hình ngắt bất đồng bộ để tối ưu độ trễ.
1. **Top-Half (Kernel)**: Nhận ngắt cứng, Ack IRQ nhanh nhất có thể và gọi `waker.wake()` tương ứng.
2. **Bottom-Half (Cell)**: Driver Cell xử lý ngắt trong một `async task`. Việc chuyển ngữ cảnh (Context Switch) được tối ưu hóa bằng cách chạy trực tiếp trong SAS.

## 4. Resource Registry (MMIO Isolation)
Trong SAS, việc hai driver cùng ghi vào một địa chỉ phần cứng là thảm họa.
* **Registry**: Kernel quản lý danh sách MMIO dựa trên **Device Tree (DTB)**.
* **Exclusive Access**: Driver phải gọi `kernel.request_mmio(base, size)`. Nếu vùng nhớ đã bị chiếm, Kernel sẽ từ chối cấp phát.

## 5. SMP & Real-Time Affinity
* **Work Stealing**: Scheduler tự động cân bằng tải giữa các core.
* **Affinity**: Các tác vụ điều khiển robot cực kỳ nhạy cảm có thể dùng `spawn_pinned(core_id)` để chiếm quyền ưu tiên tuyệt đối trên một core cụ thể, tránh bị các tác vụ AI làm gián đoạn.

## 6. Deadlock Watchdog
Vì dùng chung bộ nhớ, việc tranh chấp Lock giữa các Cell là rủi ro hiện hữu.
* **Cơ chế**: Một tác vụ nền (Low-priority task) định kỳ quét **Resource Graph**.
* **Xử lý**: Nếu phát hiện vòng lặp (Cycle), hệ thống sẽ chủ động `panic` và reload Cell có độ ưu tiên thấp nhất để giải phóng tài nguyên.