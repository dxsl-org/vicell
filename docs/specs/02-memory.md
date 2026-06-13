# ViCell Architecture: Memory Model
**Version**: 0.3 (Universal SAS & Resource Governance)
**Status**: Definitive

---

## 1. Universal SAS Layout (Trait-Based)
Thay vì hardcode địa chỉ 64-bit, ViCell dùng bộ khung **Virtual Memory Layout** trừu tượng thông qua `hal-core`.

### Layout Segments
| Segment | RV32 (Sv32) | RV64 (Sv39/48) | Đặc điểm |
| :--- | :--- | :--- | :--- |
| **Trap Zone** | Low 4KB | Low 4KB | Unmapped để bắt lỗi NULL pointers. |
| **HHDM** | Offset-based | High-half | Ánh xạ trực tiếp RAM vật lý. |
| **Kernel Static** | Fixed High | Fixed High | Code/Data của Nano Kernel. |
| **Global Heap** | Remaining | Dynamic | Vùng nhớ cấp phát cho các Cell. |

## 2. Global Allocator & Resource Governance
Hệ thống sử dụng **Hybrid Allocator** để cân bằng giữa tốc độ và chống phân mảnh.

### Quota-based Allocation (Chống "Tham")
* **Cơ chế**: Mỗi Cell có `MemoryQuota`.
* **Thực thi**: Bộ cấp phát (`GlobalAlloc`) truy vấn `CallerID` (thông qua Program Counter range) để trừ vào quỹ RAM của Cell đó.
* **OOM Policy**: Trả về `Result::Err(OutOfMemory)` thay vì panic toàn hệ thống.

### Real-Time Pool (TLSF)
Dành riêng cho các tác vụ điều khiển Robot (Tier 1). Đảm bảo thời gian cấp phát là **O(1)** và không bị block bởi các App AI nặng.

## 3. Metadata Registry & Ownership Transfer
Đây là trái tim để duy trì an toàn trong SAS khi Hot-swap.

* **Registry**: Một bảng băm theo dõi `[Address Range] -> {OwnerID, State}`.
* **State**: 
    * `Owned`: Thuộc về một Cell.
    * `AsyncLocked`: Đang trong quá trình truyền dữ liệu (DMA/Async). Không được giải phóng kể cả khi Cell sở hữu bị Unload.
* **Transfer Protocol**: Khi Cell A gửi `Box<T>` cho Cell B, Kernel cập nhật `OwnerID = B` trong Registry. Nếu A bị xóa, vùng nhớ của B vẫn an toàn.

## 4. Stack Safety (Guard Pages)
* **Cơ chế**: Mọi Stack của Task/Cell được bao bọc bởi một trang **Unmapped 4KB (Guard Page)**.
* **Hành vi**: Stack Overflow sẽ kích hoạt `Page Fault` ngay lập tức. Kernel sẽ cô lập Task đó thay vì để nó phá nát dữ liệu Cell lân cận.

## 5. Protection Policy (W^X)
Mặc dù dùng chung bộ nhớ, nhưng hardware page-level protection vẫn được bật:
* **Text**: Read + Execute (RX).
* **Data**: Read + Write (RW).
* **Read-only**: Read (R).