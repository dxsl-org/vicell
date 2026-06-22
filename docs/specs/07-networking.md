# Cellos Architecture: Networking Stack
**Version**: 0.3 (Zero-Copy User-space Stack)
**Status**: Definitive

---

## 1. Triết lý: User-space Stack trong SAS
Trong Cellos, Network Stack không nằm trong nhân (Kernel) mà là một **Service Cell** (`service-net.o`) chạy trực tiếp trong Single Address Space (SAS).

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

## 6. TLS Server Authentication (G14)

### 6.1 Tổng quan
Trước G14, net cell sử dụng `UnsecureProvider` của embedded-tls — chứng chỉ máy chủ **không được xác minh**, dễ bị tấn công MITM. G14 thay thế bằng **`ViTlsProvider`** (`cells/services/net/src/tls/provider.rs`), bọc `embedded_tls::pki::CertVerifier` để thực hiện xác thực đầy đủ theo TLS 1.3.

### 6.2 Những gì được xác minh (build mặc định `tls-roots-embedded`)
- **Chuỗi chứng chỉ (chain)**: `pki::CertVerifier` duyệt leaf → intermediates → root anchor được cấu hình.
- **Thời hạn hiệu lực**: `notBefore`/`notAfter` so sánh với đồng hồ thực từ RTC (`GetTime` op=3, epoch seconds). Giá trị được kẹp tại `Cellos_MIN_UNIX` (build-time floor) để đồng hồ chưa đặt (epoch 0) không phá vỡ kiểm tra.
- **Hostname (RFC 6125)**: khớp với `SubjectAltName` dNSName. Giới hạn của `pki.rs`: tối đa 3 dNSName, hostname ≤ 64 ký tự — endpoint vượt quá cần build `tls-roots-full`.
- **SNI rỗng bị từ chối** trước khi bắt tay (handshake).

### 6.3 Build flavors (chọn tại OS-image build time)

| Feature flag | CA anchor | Chi phí ảnh |
|---|---|---|
| `tls-roots-embedded` + `tls-ca-private` *(mặc định)* | Self-signed ECDSA P-256 fleet CA | +21 KB |
| `tls-roots-embedded` + `tls-ca-amazon` | Amazon Root CA 3 (ECDSA P-256) | +21 KB |
| `tls-roots-embedded` + `tls-ca-letsencrypt` | ISRG Root X2 (ECDSA P-384) | +100 KB |
| `tls-roots-embedded` + RSA roots | RSA opt-in (nặng) | +135 KB |
| `tls-roots-full` *(dự kiến, chưa triển khai)* | rustls-webpki multi-root cho PC/server | — |
| `tls-insecure` | `UnsecureProvider`, không xác minh | dev/lab ONLY |

`tls-insecure` in banner **`INSECURE TLS BUILD`** tại runtime. CI không được ship build này.

### 6.4 Xử lý kết nối thất bại
- Chứng chỉ không chain về anchor → kết nối thất bại (cap 0), net cell log: `connect REJECTED — certificate verification failed`.
- Timeout transport → log riêng: `transport I/O` — hai lỗi **không bao giờ bị lẫn lộn**.

### 6.5 Rủi ro còn lại (đã chấp nhận)
1. **Revocation (OCSP/CRL) ngoài phạm vi** — chuẩn ngành cho embedded (AWS/Azure IoT áp dụng tương tự). Biện pháp giảm thiểu: short-lived certs, SPKI pinning, OTA trust-anchor update.
2. **RTC manipulation**: kiểm tra hết hạn chứng chỉ chỉ đáng tin khi đồng hồ đáng tin. Clamp `Cellos_MIN_UNIX` chỉ bảo vệ khỏi đồng hồ quá sớm (epoch 0), không bảo vệ khỏi đồng hồ bị đẩy sai về phía tương lai.
3. **Không có Certificate Transparency / name-constraint / path-length** ngoài những gì `pki.rs` thực hiện.
4. **DER parser là attack surface** trên chứng chỉ thù địch; được parse bởi stack `der`/RustCrypto dưới `#![forbid(unsafe_code)]`.
5. **Chỉ TLS 1.3** — embedded-tls không hỗ trợ TLS 1.2, nên tấn công downgrade giao thức là **không thể** (đây là lợi thế tích cực).