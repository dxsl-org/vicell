# Cellos Architecture: Power Management
**Version**: 0.3 (Energy Proportionality & Pointer Swizzling)
**Status**: Definitive

---

## 1. CPU Power Governance
Cellos không sử dụng "chu kỳ chờ" (Busy-wait). Mọi giây CPU chạy đều phải có mục đích.

### Tickless Idle (Ngủ thông minh)
1. **Dự báo**: Scheduler kiểm tra thời điểm tác vụ tiếp theo cần chạy.
2. **Cấu hình**: Thiết lập Timer của phần cứng để đánh thức CPU đúng lúc đó.
3. **Thực thi**: Gọi lệnh `WFI` (Wait For Interrupt) của RISC-V để đưa nhân CPU vào trạng thái nghỉ sâu.

### DvFS (Governor Cell)
* **Cơ chế**: Một Cell chuyên trách (Governor) theo dõi tải hệ thống (Load).
* **Thực thi**: Gọi Trait `set_cpu_freq(hz)` trong HAL để điều chỉnh điện áp và tần số theo thời gian thực, giảm lãng phí nhiệt lượng khi chạy tác vụ nhẹ.

## 2. Device PM (Cooperative Lifecycle)
Mọi Driver Cell phải có trách nhiệm với năng lượng mà thiết bị của nó tiêu thụ.

```rust
pub trait Powerable {
    fn suspend(&mut self) -> Result<()>;
    fn resume(&mut self) -> Result<()>;
}
```

**Power Manager Cell**: Duy trì biểu đồ phụ thuộc (Dependency Graph) để tắt/mở thiết bị theo đúng thứ tự (ví dụ: tắt App -> tắt Bus -> tắt CPU).

## 3. System States & The Pointer Challenge
SAS gặp khó khăn lớn khi Hibernate (S4) vì nó chứa hàng triệu địa chỉ bộ nhớ tuyệt đối.

**Pointer Swizzling (Đóng gói địa chỉ)**
Khi Hibernate, hệ thống không thể chỉ "dump" RAM ra đĩa vì khi khởi động lại, các Cell có thể được nạp vào địa chỉ khác.

**Deflation (Nén)**: Kernel quét Metadata Registry, chuyển đổi mọi con trỏ tuyệt đối (0x1234...) thành địa chỉ tương đối: Ref(CellID, Section, Offset).

**Persistence**: Lưu trạng thái đã "nén" này xuống đĩa cùng với Snapshot của Heap.

**Reflation (Bung)**: Khi Resume, thực hiện The Great Re-linking. Linker vá lại các địa chỉ tương đối thành địa chỉ tuyệt đối mới dựa trên vị trí thực tế của các Cell vừa được nạp lại.

## 4. Tối ưu cho Robot Nano (RV32)
Vì các thiết bị RV32 thường không có RAM dự phòng lớn, Cellos áp dụng chính sách Aggressive Suspend:

**Immediate Sleep**: Ngay khi không có tác vụ nào trong RunQueue, CPU vào trạng thái nghỉ ngay lập tức.

**Peripheral Power Gating**: Kernel tự động ngắt nguồn cho các ngoại vi (như cảm biến) nếu Driver Cell tương ứng không có yêu cầu I/O nào trong một khoảng thời gian xác định.

## 5. Thermal Awareness
Trong các thiết bị nhỏ kín khí, nhiệt độ là kẻ thù của hiệu năng.

**Thermal Cell**: Theo dõi cảm biến nhiệt độ qua HAL.

**Throttling**: Nếu nhiệt độ vượt ngưỡng, Power Manager chủ động hạ tần số CPU hoặc tạm dừng các tác vụ Tier 2/Tier 3 không quan trọng để hạ nhiệt.