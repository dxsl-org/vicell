# ViOS Architecture: VFS & Filesystems
**Version**: 0.4 (Zero-Copy I/O, Dual-VFS & B-tree TFS Integration)
**Status**: Definitive


## 1. VFS Contract (The Interface)

Trong ViOS, VFS là một bộ các **Traits** định nghĩa trong `libs/api` để đảm bảo tính hoán đổi giữa các bản Implementation.

* **Stable ABI**: Các Trait được bọc trong `#[repr(C)]` để đảm bảo ứng dụng không bị vỡ khi nâng cấp Cell hệ thống.
* **Async by Design**: Mọi thao tác I/O đều trả về `Future`, tận dụng tối đa mô hình phi tập trung của ViOS.

```rust
pub trait ViFileSystem: Send + Sync {
    fn open(&self, path: &str) -> ViResult<Box<dyn ViFile>>;
    fn mount(&self, target: &str) -> ViResult<()>;
}

pub trait ViFile: Send + Sync {
    // Quy tắc "Owned Buffers ONLY": Quyền sở hữu buffer chuyển giao hoàn toàn qua SAS
    fn read(&mut self, buf: Box<[u8]>) -> Future<ViResult<(usize, Box<[u8]>)>>;
    fn write(&mut self, buf: Box<[u8]>) -> Future<ViResult<(usize, Box<[u8]>)>>;
}

```


## 2. Dual-VFS Strategy (FS Cells)

ViOS triển khai cơ chế VFS kép tùy chọn (Switchable) để tối ưu hóa cho từng mục đích sử dụng.

### **viFS1: Classic Layer (RedoxFS Fork)**

* **Nguồn gốc**: Fork từ RedoxFS.
* **Mục tiêu**: Cung cấp khả năng tương thích POSIX-like cho các App Cell được port từ môi trường Unix sang.
* **Đặc điểm**: Dựa trên cấu trúc Node và Path truyền thống, ổn định và dễ triển khai.

### **viFS2: Native Layer (TFS - Tree File System)**

* **Nguồn gốc**: Dựa trên cấu trúc B-tree hiện đại.
* **Mục tiêu**: Tối ưu hóa tuyệt đối cho kiến trúc SAS, phục vụ các tác vụ nạp Cell và xử lý dữ liệu Jarvis tốc độ cao.
* **Đặc điểm**: Hỗ trợ Snapshot, nén dữ liệu, và tìm kiếm Metadata với độ phức tạp  nhờ cấu trúc cây.

### **Storage Drivers**

* **FAT32**: Sử dụng crate `fatfs` cho các phân vùng khởi động và thẻ nhớ nhỏ.
* **exFAT**: Hỗ trợ thẻ nhớ dung lượng lớn (>32GB) cho các robot nano quay phim/chụp ảnh.


## 3. Cơ chế Direct I/O & Zero-Copy
Nhờ lợi thế của Single Address Space (SAS), ViOS đạt được tốc độ đọc/ghi dữ liệu ở mức vật lý mà không cần memcpy.

1. **Cấp phát**: App Cell cấp phát một buffer (`Box<[u8]>`) từ Global Heap.
2. **Chuyển giao**: Khi gọi `read()`, quyền sở hữu buffer chuyển từ App sang VFS Cell, sau đó sang Disk Driver Cell thông qua cơ chế **Grant** trong Task TCB.
3. **DMA**: Disk Driver dịch địa chỉ ảo của buffer sang địa chỉ vật lý và ra lệnh cho phần cứng ghi dữ liệu trực tiếp.
4. **Hoàn tất**: Driver trả lại quyền sở hữu buffer cho App kèm theo số lượng byte đã đọc.


## 4. Global Page Cache & SAS Optimization

Thay vì mỗi FS Cell tự giữ cache, ViOS dùng một Unified Page Cache nằm trong vùng nhớ dùng chung của SAS.

* **Zero-copy Metadata**: VFS2 (TFS) có thể trả về con trỏ trực tiếp đến các cấu trúc B-tree trong RAM, cho phép App đọc Metadata mà không cần thông qua syscall trung gian.
* **LRU & OOM**: Chính sách thu hồi bộ nhớ (Eviction) được quản lý tập trung bởi Kernel để tránh xung đột tài nguyên giữa các Cell.


## 5. Large File Support (LFS) trên Multi-Arch
Mặc dù hỗ trợ cả RV32, nhưng VFS của ViOS mặc định dùng 64-bit offsets (`u64`).

* **RV32 (Robot Nano)**: Sử dụng các cặp thanh ghi (Register Pairs) để xử lý offsets > 4GB, đảm bảo robot có thể ghi video vào thẻ exFAT dung lượng lớn.
* **RV64/RV128 (Jarvis)**: Tận dụng trực tiếp độ rộng thanh ghi tự nhiên cho hiệu suất tối đa.


## 6. Fault Isolation (Luật 4 & Panic Recovery)
Mỗi FS Cell được bao bọc bởi cơ chế an toàn của Kernel:

* **Isolation**: Nếu VFS1 (RedoxFS) bị panic, chỉ các App đang truy cập phân vùng đó bị ảnh hưởng.
* **Recovery**: Kernel thực hiện reload FS Cell. Nhờ SAS, các cấu trúc dữ liệu trong Page Cache vẫn có thể được giữ lại để Cell mới tái sử dụng (Warm Reboot).