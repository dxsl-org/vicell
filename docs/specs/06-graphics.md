# ViOS Architecture: Graphics & Input
**Version**: 0.3 (Zero-Cost Compositing & Low-Latency Input)
**Status**: Definitive

---

## 1. Triết lý Đồ họa: Shared Framebuffer
Trong ViOS SAS, chúng ta loại bỏ hoàn toàn việc copy buffer giữa Client và Server (như X11/Wayland).

### Quy trình hiển thị Zero-Copy
1. **Compositor Cell**: Nắm giữ con trỏ đến **Physical Framebuffer** do phần cứng cung cấp.
2. **App Cells**: Vẽ vào các vùng nhớ riêng gọi là **Surface**.
3. **Compositing**: 
    * Thay vì copy toàn bộ, Compositor chỉ thực hiện `memcpy` các vùng dữ liệu bị thay đổi (Damaged regions).
    * **Game/Full-screen Mode**: Compositor chuyển nhượng trực tiếp quyền sở hữu vùng nhớ Framebuffer cho App Cell thông qua Capability. Đây là mức hiệu năng **True Zero-Copy**.

## 2. Hệ thống Input: Latency-Free Dispatcher
Độ trễ từ lúc chạm/gõ đến lúc App nhận được sự kiện phải bằng 0.

* **Input Driver (Tier 1)**: Nhận ngắt (IRQ), giải mã thành `InputEvent` (Enum).
* **Dispatcher**: 
    * Nắm giữ danh sách các `Window` của các Cell.
    * Xác định Cell đang được focus.
    * **Direct Call**: Gọi trực tiếp hàm `on_event(event)` của Cell đó mà không qua hàng đợi trung gian (Queue) của OS truyền thống.



## 3. Chế độ vận hành (Profiles)
ViOS cho phép cấu hình linh hoạt tùy theo mục đích sử dụng:

| Mode | Target | Đồ họa |
| :--- | :--- | :--- |
| **Mode 1: CLI** | Server / Robot Nano | Không GUI. Chỉ dùng Serial/VGA Driver cho Shell. |
| **Mode 2: Kiosk** | Industrial Panel / ATM | Full-screen cho một App duy nhất. Tối ưu Direct Scanout. |
| **Mode 3: Desktop** | Workstation | Hỗ trợ nhiều cửa sổ, Taskbar, Start Menu thông qua Slint. |

## 4. UI Toolkit: Slint Standard
Để tránh phân mảnh và tối ưu tài nguyên, ViOS chuẩn hóa trên **Slint**.
* **Native Integration**: Slint được tích hợp sâu vào `ostd`, hỗ trợ cả phần cứng không có tăng tốc GPU (Software Rendering).
* **Resource Friendly**: Phù hợp cho cả các bảng điều khiển robot nhỏ chạy RV64 nhưng RAM hạn chế.

## 5. Bảo mật đồ họa trong SAS
Vì các Cell dùng chung bộ nhớ, chúng ta sử dụng **Tokens (Capabilities)** để bảo vệ:
* App Cell A không thể đọc vùng nhớ Surface của App Cell B trừ khi được Compositor cấp quyền.
* Mọi hành vi truy cập trái phép vùng nhớ đồ họa sẽ kích hoạt `Page Fault` và khiến Cell vi phạm bị **Poisoned** ngay lập tức.