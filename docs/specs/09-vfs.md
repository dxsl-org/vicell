# Cellos Architecture: VFS & Filesystems
**Version**: 0.5 (Mount-Table Layered Backends — thay thế Dual-VFS)
**Status**: Definitive
**Changed 2026-06-10**: Bỏ chiến lược Dual-VFS (viFS1 RedoxFS fork / viFS2 TFS — TFS upstream đã ngừng phát triển từ ~2018). Thay bằng mô hình MountTable một VFS service + nhiều backend cắm song song. Plan chi tiết: `.agents/260610-1202-vfs-mount-table-backends/`.


## 1. VFS Contract (The Interface)

Trong Cellos, VFS là một bộ các **Traits** định nghĩa trong `libs/api` để đảm bảo tính hoán đổi giữa các bản Implementation.

* **Stable ABI**: Các Trait được bọc trong `#[repr(C)]` để đảm bảo ứng dụng không bị vỡ khi nâng cấp Cell hệ thống.
* **Async by Design**: Mọi thao tác I/O đều trả về `Future`, tận dụng tối đa mô hình phi tập trung của Cellos.

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


## 2. Mount-Table Layered Backends

Một VFS service duy nhất (`cells/services/vfs/`) sở hữu một **MountTable** (longest-prefix match, `mount.rs`). Mỗi backend phụ trách một prefix, **ngang hàng** chứ không xếp chồng. Backend mới cắm thêm qua `Arc<dyn ViFileSystem>` mà không đụng các backend hiện có.

```
VFS service (MountTable, longest-prefix match)
├── /bin, /etc   → BootFS  (initramfs — FAT16 nhúng kernel, read-only)
├── /tmp         → RamFS   (tmpfs, volatile, read-write)
├── /data        → FAT32 hiện tại → littlefs (power-safe, G1 tail)
├── /mnt/sd      → FAT32   (interop thẻ SD/PC — sau khi /data chuyển littlefs)
└── /srv         → Native FS (CoW, checksum — G2, cùng NVMe)
```

### Vai trò từng backend

| Backend | Prefix | Vai trò | Giai đoạn |
|---|---|---|---|
| **BootFS** (kernel `VIFS1`, `kernel_fs.img` FAT16 nhúng) | `/bin`, `/etc` | Initramfs: giải bài toán con gà–quả trứng (kernel load được binary VFS service trước khi VFS tồn tại). Loader fallback tại `kernel/src/loader/early.rs` | Có rồi |
| **RamFS** | `/tmp` | tmpfs chuẩn — scratch space volatile | Có rồi |
| **FAT32** (crate `fatfs`, có LFN/VFAT) | `/data` (hiện tại) → `/mnt/sd` | Interop thẻ SD ≤32GB / boot partition RPi / trao đổi dữ liệu với PC. **Không journaling — không dùng làm persistent store chính cho robot** | Có rồi |
| **littlefs** | `/data` | Persistent store power-loss-resilient cho config/log/model — mất điện giữa chừng ghi không hỏng volume. Bắt buộc trước robot demo trên board thật | G1 tail |
| **Native FS** | `/srv` | CoW + checksum cho server workload. **Quyết định: RedoxFS port** (MIT, ~10 K LOC; xem [ADR](09b-vfs-native-fs-adr.md)). Implement tại G2 cùng NVMe. Hiện stub `StubBackend` mounted tại `/srv` — trả empty/false, không crash VFS | G2 |
| **exFAT** | `/mnt/sd` (mở rộng) | Thẻ SDXC >32GB nguyên bản (xuất xưởng exFAT, `fatfs` không đọc được). `FatBackend::mount()` tự detect OEM-Name `"EXFAT   "` và log cảnh báo rõ thay vì lỗi cryptic. Full support chỉ khi có nhu cầu thật | Theo nhu cầu |

### Quyết định đã chốt (2026-06-10)

* ❌ **Dual-VFS viFS1/viFS2 bị loại bỏ**: TFS upstream đã chết; RedoxFS port (~10K+ LOC) quá lớn so với nhu cầu G1 (YAGNI).
* ⚠️ **Xung đột tên đã gỡ**: thuật ngữ "viFS1" trong spec cũ (= RedoxFS fork) bị bỏ; `VIFS1` trong kernel (`kernel/src/fs.rs`) từ nay hiểu là **BootFS/initramfs**.
* 🔧 **Tech debt ghi nhận**: VFS service hiện `include_bytes!` lại các ELF `/bin` (nhúng trùng lặp với `kernel_fs.img`) — sẽ thay bằng proxy qua syscall kernel (xem plan).


## 3. Cơ chế Direct I/O & Zero-Copy
Nhờ lợi thế của Single Address Space (SAS), Cellos đạt được tốc độ đọc/ghi dữ liệu ở mức vật lý mà không cần memcpy.

1. **Cấp phát**: App Cell cấp phát một buffer (`Box<[u8]>`) từ Global Heap.
2. **Chuyển giao**: Khi gọi `read()`, quyền sở hữu buffer chuyển từ App sang VFS Cell, sau đó sang Disk Driver Cell thông qua cơ chế **Grant** trong Task TCB.
3. **DMA**: Disk Driver dịch địa chỉ ảo của buffer sang địa chỉ vật lý và ra lệnh cho phần cứng ghi dữ liệu trực tiếp.
4. **Hoàn tất**: Driver trả lại quyền sở hữu buffer cho App kèm theo số lượng byte đã đọc.


## 4. Global Page Cache & SAS Optimization

Thay vì mỗi FS Cell tự giữ cache, Cellos dùng một Unified Page Cache nằm trong vùng nhớ dùng chung của SAS.

* **Zero-copy Metadata**: backend native (G2) có thể trả về con trỏ trực tiếp đến cấu trúc metadata trong RAM, cho phép App đọc Metadata mà không cần syscall trung gian.
* **LRU & OOM**: Chính sách thu hồi bộ nhớ (Eviction) được quản lý tập trung bởi Kernel để tránh xung đột tài nguyên giữa các Cell.


## 5. Large File Support (LFS) trên Multi-Arch
Mặc dù hỗ trợ cả RV32, nhưng VFS của Cellos mặc định dùng 64-bit offsets (`u64`).

* **RV32 (Robot Nano)**: Sử dụng các cặp thanh ghi (Register Pairs) để xử lý offsets > 4GB, đảm bảo robot có thể ghi video vào thẻ exFAT dung lượng lớn.
* **RV64/RV128 (Jarvis)**: Tận dụng trực tiếp độ rộng thanh ghi tự nhiên cho hiệu suất tối đa.


## 6. Fault Isolation (Luật 4 & Panic Recovery)
Mỗi FS Cell được bao bọc bởi cơ chế an toàn của Kernel:

* **Isolation**: Nếu một FS backend bị panic, chỉ các App đang truy cập phân vùng đó bị ảnh hưởng.
* **Recovery**: Kernel thực hiện reload FS Cell. Nhờ SAS, các cấu trúc dữ liệu trong Page Cache vẫn có thể được giữ lại để Cell mới tái sử dụng (Warm Reboot).