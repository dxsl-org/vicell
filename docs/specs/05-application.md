# ViOS Architecture: Application Tiers
**Version**: 0.3 (Multi-Tier Isolation & Legacy Support)
**Status**: Definitive

---

## 1. Chiến lược phân tầng (The Tiered Strategy)
ViOS phân cấp ứng dụng dựa trên sự cân bằng giữa **Hiệu năng** và **Tính an toàn**.

| Đặc điểm | Tier 1: Native | Tier 2: Managed | Tier 3: Virtual |
| :--- | :--- | :--- | :--- |
| **Công nghệ** | Rust `.o` (SAS) | WASM / POSIX | Hypervisor Cell |
| **Hiệu năng** | 100% Native | ~95% Native | 85-90% Native |
| **Cách ly** | Compiler (LBI) | Software Sandbox | Hardware (Stage-2) |

## 2. Tier 1: Native Cells (The "Metal" Layer)
Dành cho các thành phần cốt lõi và yêu cầu thời gian thực (Real-time).
* **Thực thi**: Chạy trực tiếp trong SAS (Ring 0/S-Mode).
* **Ứng dụng**: Drivers, FileSystems, Cloud Microservices, Điều khiển Robot.
* **An toàn**: Bắt buộc tuân thủ `#![forbid(unsafe_code)]` và phải có chữ ký số (Signed Cells).

## 3. Tier 2: Managed Cells (Middleware)
Dành cho ứng dụng bên thứ ba và logic có tính di động cao.
* **Thực thi**: Mã nguồn được biên dịch sang WASM. 
* **Tính di động**: Cùng một file `.wasm` có thể chạy trên cả RV32 và RV64 mà không cần biên dịch lại.
* **Cách ly**: WASM Validator đảm bảo code không thể truy cập trái phép bộ nhớ ngoài vùng được cấp (Linear Memory).

## 4. Tier 3: Virtualization (Legacy & Sensitive Silos)
Cung cấp khả năng tương thích ngược và bảo mật phần cứng tuyệt đối.
* **Cơ chế**: Sử dụng **Hypervisor Cell** để tạo các máy ảo (VM).
* **Hardware Isolation**: Sử dụng **Stage-2 Paging** (Guest Physical -> Host Physical) để tạo rào cản phần cứng thực sự.
* **Trường hợp sử dụng**:
    1.  **Legacy**: Chạy các ứng dụng Linux, Windows hoặc Android chưa được port sang ViOS.
    2.  **Security (Sensitive Silos)**: Các tác vụ xử lý khóa bí mật (Private Keys) hoặc dữ liệu cực kỳ nhạy cảm phải chạy ở Tier 3 để chống lại các cuộc tấn công kênh kề (Side-channel/Spectre) vốn là điểm yếu của mô hình SAS.



## 5. Cấu hình theo thiết bị (Platform Profiles)
* **ViOS-Nano (RV32)**: Chỉ hỗ trợ **Tier 1** và **Tier 2** để tối ưu RAM và Pin. Lược bỏ Tier 3.
* **ViOS-Standard (RV64)**: Hỗ trợ đầy đủ **3 Tầng**, cho phép chạy song song Robot Control (Tier 1) và Linux App (Tier 3).