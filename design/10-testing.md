# ViOS Architecture: Testing & Verification
**Version**: 0.3 (SAS-Specific Quality Assurance)
**Status**: Definitive

---

## 1. Triết lý: Test trong "Nồi lẩu" SAS
Trong mô hình Single Address Space, một lỗi nhỏ có thể phá hủy toàn bộ hệ thống. Do đó, testing không chỉ là kiểm tra logic mà là kiểm tra ranh giới (Boundaries).

## 2. Các tầng Testing
1. **KUnit (In-Kernel Unit Tests)**:
    * Chạy trực tiếp bên trong Ring 0 (QEMU hoặc Hardware).
    * Kiểm tra các Trait thực thi của `hal/core` và các logic lõi của Nano Kernel.
2. **Cell Integration Tests**:
    * Mô phỏng việc nạp/gỡ (Load/Unload) các Cell liên tục để kiểm tra rò rỉ bộ nhớ trong `Metadata Registry`.
3. **SASan (Single Address Space Sanitizer)**:
    * Công cụ dùng lúc debug để phát hiện một Cell cố tình truy cập vào vùng nhớ của Cell khác mà không có quyền sở hữu (Ownership).

## 3. Fault Injection Cell (Kẻ phá hoại)
Một Cell đặc biệt được thiết kế để:
* Gây ra **Panic** ngẫu nhiên trong các callback để test cơ chế `catch_unwind`.
* Chiếm dụng 99% RAM để test cơ chế **Memory Quota**.
* Gây ra **Deadlock** để test bộ phận giám sát (Watchdog).

## 4. Hardware-in-the-loop (HITL)
Đặc biệt quan trọng cho Robot:
* Kiểm tra độ trễ phản hồi (Latency) từ lúc có IRQ thực tế đến khi Driver Cell nhận được `Waker`.
* Kiểm tra việc tiêu thụ năng lượng của các Task trong trạng thái **Tickless Idle**.