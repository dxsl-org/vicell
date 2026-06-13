# ViCell Architecture: Networking Stack
**Version**: 0.3 (Zero-Copy User-space Stack)
**Status**: Definitive

---

## 1. Triết lý: User-space Stack trong SAS
Trong ViCell, Network Stack không nằm trong nhân (Kernel) mà là một **Service Cell** (`service-net.o`) chạy trực tiếp trong Single Address Space (SAS).

* **Hiệu năng**: Loại bỏ chi phí chuyển đổi ngữ cảnh (Context Switch) giữa User-space và Kernel-space của OS truyền thống.
* **Thư viện**: Sử dụng **smoltcp** (Native Rust, `no_std`) — ngăn xếp TCP/IP hướng sự kiện, không cần cấp phát bộ nhớ động (Heap-less).

## 2. Các thành phần chính
1. **Driver Cell** (e.g., `driver-virtio-net`):
    * Quản lý các vòng đệm phần cứng (RX/TX Rings).
    * Thực thi Trait `PHY` để giao tiếp với Stack.
2. **Stack Cell** (`service-net`):
    * Quản lý máy trạng thái TCP, xử lý IP/UDP/ICMP.
    * Quản lý các cổng (Ports) và điều phối gói tin.
    * Expose Trait `TcpStack` cho các App Cells khác.

## 3. Luồng gói tin Zero-Copy (Bảo hiểm cho SAS)
Tận dụng cơ chế **Owned Buffers** từ File 03 để đảm bảo an toàn tuyệt đối mà không cần sao chép:

1. **NIC -> RAM**: Phần cứng DMA ghi gói tin trực tiếp vào `PacketBuffer` (RAM).
2. **Driver -> Stack**: Chuyển quyền sở hữu con trỏ `Box<PacketBuffer>` qua một lời gọi hàm trực tiếp. **Không `memcpy`**.
3. **Stack -> App**:
    * Stack xử lý các Header tại chỗ trên buffer đó.
    * App sử dụng cơ chế **Peek** hoặc **Take** để lấy dữ liệu Payload trực tiếp. Quyền sở hữu buffer được trả lại Driver sau khi App dùng xong hoặc theo chu kỳ xoay vòng.



## 4. Socket API & ostd Integration
App Cell không cần biết sự phức tạp bên dưới, chỉ cần dùng API chuẩn thông qua `ostd`.

```rust
pub trait TcpStack {
    fn connect(&self, addr: IpEndpoint) -> Result<Box<dyn TcpStream>>;
    fn listen(&self, port: u16) -> Result<Box<dyn TcpListener>>;
}
```
**Async Ready**: Các hàm trả về Future, cho phép hàng ngàn kết nối đồng thời với bộ nhớ tối thiểu.

**Multi-Arch**: Code của service-net là 100% Rust no_std, biên dịch chung cho cả RV32 (Nano Robot) và RV64 (Jarvis).

## 5. Bảo mật & Cô lập mạng
**Port Ownership**: Mỗi Port khi mở được gán một OwnerID trong Metadata Registry.

**Capability Check**: Chỉ các Cell có NetworkCap mới được phép khởi tạo kết nối ra ngoài hoặc mở Port lắng nghe.

**Fault Recovery**: Nếu Stack Cell bị panic, Kernel thực hiện Reload Cell đó. Các App Cell sẽ nhận lỗi ConnectionReset và có thể tự động kết nối lại sau khi Stack hồi sinh.