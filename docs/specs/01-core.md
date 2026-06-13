# ViCell Architecture: Core System
**Version**: 0.3 (Cellular SAS - Enhanced Integrity)
**Status**: Definitive

---

## 1. System Philosophy
ViCell dịch chuyển từ cách ly bằng phần cứng sang **Language-Based Isolation (LBI)** để triệt tiêu chi phí IPC.

### Key Differentiators
| Feature | Traditional OS | **ViCell Cellular** |
| :--- | :--- | :--- |
| **Isolation** | Hardware MMU (Slow) | **Compiler/Language (Zero-Cost)** |
| **IPC** | Message Passing | **Direct Function Call** |
| **Kernel Role** | Resource Manager | **Runtime Linker & Manager** |

## 2. The Cellular Model (SAS)
Tất cả chạy trong **Single Address Space (Ring 0)**.

### The "Cell"
* **Dạng vật lý**: File ELF (.o) đã được ký số.
* **Liên kết**: Trực tiếp qua VTable hoặc Symbol Table.

## 3. Nano Kernel: The Construction Site
Kernel tối giản, tập trung vào việc "xây dựng" hệ thống lúc runtime.

### Global Symbol Table (Enhanced)
* **Cấu trúc**: Sử dụng **Lock-free Hash Table** để ánh xạ `SymbolName -> Address`.
* **Tốc độ**: O(1) lookup, đảm bảo nạp hàng trăm Cell trong < 500ms.

### Dependency Management (DAG & Weak Refs)
Để tránh Deadlock khi Unload, ViCell phân loại liên kết:
1.  **Strong Ref**: Cell A không thể sống thiếu Cell B. `ref_count` tăng.
2.  **Weak Ref**: Liên kết tạm thời (như Logging). Không tăng `ref_count`, cho phép Unload Cell đích và trả về lỗi `SymbolNotFound` khi gọi.

## 4. The Gatekeeper & Security
1.  **Signature**: Mọi Cell phải có chữ ký Ed25519 từ ViCell Lab.
2.  **Capabilities (Tokens)**: Sử dụng Zero-Sized Types (ZST). 
    * `fn reboot(_: RebootCap)`.
    * Token chỉ được cấp qua hàm `init()` của Cell và không thể copy trái phép.

## 5. Fault Tolerance (Panic Recovery)
* **Unwind Boundary**: Kernel wrap mọi inter-cell call bằng `catch_unwind`.
* **Safe Recovery**: Khi Cell panic, Kernel:
    1.  Cô lập Cell (Trạng thái Poisoned).
    2.  Nếu Cell là Driver, thực hiện reset phần cứng tương ứng.
    3.  Tải bản copy mới từ Disk và thực hiện **Re-linking** nóng.

## 6. Lifecycle Integrity
* **Hot-swap**: Chỉ cho phép khi không có Strong Ref nào đang hoạt động.
* **Zombie State**: Đánh dấu Cell đang chờ chết, từ chối mọi yêu cầu liên kết mới.