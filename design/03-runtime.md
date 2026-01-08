# ViOS Architecture: Runtime & SDK
**Version**: 0.3 (Zero-Copy Inter-Cell Communication)
**Status**: Definitive

---

## 1. IPC: Direct Method Calls
Trong ViOS, khái niệm IPC truyền thống bị loại bỏ. Mọi tương tác giữa các Cell là **gọi hàm trực tiếp (Direct Call)** thông qua Rust Traits.

* **Performance**: Chi phí tương đương một lời gọi hàm ảo (~2-3 chu kỳ CPU).
* **Interface**: Định nghĩa trong crate `libs/api`. Sử dụng `#[repr(C)]` cho các cấu trúc dữ liệu ở biên giới (Boundaries) để đảm bảo **Stable ABI**.
* **Data Flow**: Mặc định là **Zero-copy**. Dữ liệu được truyền dưới dạng tham chiếu (`&T`) hoặc quyền sở hữu (`Box<T>`).

## 2. Async/Await & Safety (Owned Buffers)
ViOS tận dụng triệt để mô hình lập trình bất đồng bộ của Rust để tối ưu I/O.

### Quy tắc "Owned Buffers ONLY"
Để ngăn chặn lỗi ghi đè bộ nhớ khi một Cell bị unload đột ngột:
* **Quy tắc**: Cấm truyền `&mut [u8]` qua ranh giới Async giữa các Cell.
* **Giải pháp**: Phải truyền `Box<[u8]>` hoặc `Vec<u8>`. Quyền sở hữu (Ownership) được chuyển giao hoàn toàn cho Driver.

### Async Pinning Registry (Lá chắn Unload)
* **Cơ chế**: Khi một vùng nhớ đang tham gia vào tác vụ Async, nó được đánh dấu là **Pinned/Locked** trong `Metadata Registry`.
* **Bảo vệ**: Kernel sẽ từ chối lệnh `unload` của Cell sở hữu ban đầu cho đến khi tác vụ Async hoàn tất và quyền sở hữu được trả về hoặc giải phóng.

## 3. Hot-Swap & State Transfer
ViOS hỗ trợ nâng cấp phần mềm mà không cần ngừng hệ thống (Live Update).

* **Protocol**: Các Cell quan trọng phải thực thi Trait `StateTransfer`.
* **Quy trình**:
    1. Kernel đóng băng (Pause) các luồng thực thi của `OldCell`.
    2. Gọi `serialize_state()` để trích xuất dữ liệu trạng thái.
    3. Nạp `NewCell` và gọi `deserialize_state(blob)`.
    4. Tráo đổi con trỏ hàm (Symbol Re-linking) và giải phóng `OldCell`.

## 4. Boot Optimization (Instant On)
Để robot khởi động < 1 giây, ViOS sử dụng cơ chế **Heap Snapshotting**.

* **Pre-linked Image**: Toàn bộ trạng thái bộ nhớ sau khi nạp và link các Cell cơ bản được lưu thành file ảnh (`system.img`).
* **Fast Boot**: Kernel chỉ cần nạp ảnh này trực tiếp vào RAM tại một địa chỉ cố định. Bỏ qua bước giải mã ELF và Re-linking lúc khởi động.

## 5. Tooling: `ostd` & `cargo-vios`
* **`ostd`**: Thư viện chuẩn thay thế `std`, cung cấp các interface cho Allocator, Async Runtime và Logging.
* **Multi-Arch Build**: `cargo-vios` hỗ trợ biên dịch song song cho nhiều Target (RV32, RV64) từ cùng một mã nguồn thông qua cơ chế `hal/core`.