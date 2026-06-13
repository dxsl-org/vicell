# ViCell Architecture: Runtime & SDK
**Version**: 0.3 (Zero-Copy Inter-Cell Communication)
**Status**: Definitive

---

## 1. IPC: Direct Method Calls
Trong ViCell, khái niệm IPC truyền thống bị loại bỏ. Mọi tương tác giữa các Cell là **gọi hàm trực tiếp (Direct Call)** thông qua Rust Traits.

* **Performance**: Chi phí tương đương một lời gọi hàm ảo (~2-3 chu kỳ CPU).
* **Interface**: Định nghĩa trong crate `libs/api`. Sử dụng `#[repr(C)]` cho các cấu trúc dữ liệu ở biên giới (Boundaries) để đảm bảo **Stable ABI**.
* **Data Flow**: Mặc định là **Zero-copy**. Dữ liệu được truyền dưới dạng tham chiếu (`&T`) hoặc quyền sở hữu (`Box<T>`).

## 2. Async/Await & Safety (Owned Buffers)
ViCell tận dụng triệt để mô hình lập trình bất đồng bộ của Rust để tối ưu I/O.

### Quy tắc "Owned Buffers ONLY"
Để ngăn chặn lỗi ghi đè bộ nhớ khi một Cell bị unload đột ngột:
* **Quy tắc**: Cấm truyền `&mut [u8]` qua ranh giới Async giữa các Cell.
* **Giải pháp**: Phải truyền `Box<[u8]>` hoặc `Vec<u8>`. Quyền sở hữu (Ownership) được chuyển giao hoàn toàn cho Driver.

### Async Pinning Registry (Lá chắn Unload)
* **Cơ chế**: Khi một vùng nhớ đang tham gia vào tác vụ Async, nó được đánh dấu là **Pinned/Locked** trong `Metadata Registry`.
* **Bảo vệ**: Kernel sẽ từ chối lệnh `unload` của Cell sở hữu ban đầu cho đến khi tác vụ Async hoàn tất và quyền sở hữu được trả về hoặc giải phóng.

## 3. Hot-Swap & State Transfer
ViCell hỗ trợ nâng cấp phần mềm mà không cần ngừng hệ thống (Live Update).

* **Protocol**: Các Cell quan trọng phải thực thi Trait `StateTransfer`.
* **Quy trình**:
    1. Kernel đóng băng (Pause) các luồng thực thi của `OldCell`.
    2. Gọi `serialize_state()` để trích xuất dữ liệu trạng thái.
    3. Nạp `NewCell` và gọi `deserialize_state(blob)`.
    4. Tráo đổi con trỏ hàm (Symbol Re-linking) và giải phóng `OldCell`.

## 4. Boot Optimization (Instant On)
Để robot khởi động < 1 giây, ViCell sử dụng cơ chế **Heap Snapshotting**.

### 4.1 Mục tiêu
- **Cold boot** (lần đầu hoặc sau update): parse ELF + link + init cells → ~2–5 giây
- **Warm boot** (snapshot valid): load `system.img` trực tiếp vào RAM → **< 100 ms**

### 4.2 Cơ chế hoạt động

```
Cold Boot:
  Limine → Kernel init → ELF parse cells → Link vtables → Init all cells
                                                              ↓
                                                   serialize_snapshot()
                                                              ↓
                                                   ghi system.img ra FAT16

Warm Boot:
  Limine → Kernel init → kiểm tra system.img header
                              ↓ valid
                        mmap system.img → physical RAM
                              ↓
                        restore vtable pointers (PA-relative patch)
                              ↓
                        reinit VirtIO devices (MMIO không snapshot)
                              ↓
                        resume cells từ saved entry point
```

### 4.3 Snapshot Format (`system.img`)

```
Offset  Size   Field
0x00    8      Magic: b"ViCell_SNP"
0x08    4      Version: u32 (LE)
0x0C    4      CRC32 of entire image (field = 0 during calculation)
0x10    8      Kernel build hash (SHA256 first 8 bytes)
0x18    8      Cell table hash (SHA256 of /bin/ contents)
0x20    8      Physical load address of snapshot region
0x28    8      Total snapshot size in bytes
0x30    N      Page frames: raw physical memory content
0x30+N  M      Relocation table: [(va_offset: u32, pa_base: u32)] for vtable patches
```

**Invalidation**: Snapshot stale (fallback to cold boot) nếu:
- Kernel build hash thay đổi (recompile kernel)
- Cell table hash thay đổi (bất kỳ cell nào trong /bin/ bị cập nhật)
- CRC32 mismatch (corruption)

### 4.4 Ràng buộc triển khai

| Ràng buộc | Lý do |
|-----------|-------|
| VirtIO devices **không** được snapshot | MMIO registers reset sau power cycle; phải reinit |
| MMIO regions bị exclude khỏi page frame dump | Ghi vào MMIO có side effect (gửi packet, eject disk) |
| Snapshot dùng **physical addresses** (PA) | VA có thể thay đổi nếu KASLR kích hoạt; PA stable |
| KASLR + Snapshotting: tương thích qua PA-relative reloc table | Kernel áp dụng VA randomization *sau* khi load snapshot |
| Stack của mỗi Cell **không** được snapshot | Stack chứa return addresses VA; bị invalidate sau KASLR |
| Heap và global data: snapshot đầy đủ | Owned buffers, vtables, static config — safe để restore |

### 4.5 Prerequisites trước khi triển khai (Phase 29)

- [ ] **Metadata Registry** hoàn chỉnh — cần biết OwnerID của từng page để exclude MMIO
- [ ] **Direct IPC vtable** (Phase 27) — snapshot cần capture vtable layout, không phải syscall table
- [ ] **FAT16 write path** ổn định — ghi `system.img` sau cold boot
- [ ] **Fixed physical layout** đã confirmed (no physical ASLR)
- [ ] Snapshot size estimate: ~4–8 MB cho kernel + 6 base cells (tùy heap usage)

## 5. Tooling: `ostd` & `cargo-ViCell`
* **`ostd`**: Thư viện chuẩn thay thế `std`, cung cấp các interface cho Allocator, Async Runtime và Logging.
* **Multi-Arch Build**: `cargo-ViCell` hỗ trợ biên dịch song song cho nhiều Target (RV32, RV64) từ cùng một mã nguồn thông qua cơ chế `hal/core`.