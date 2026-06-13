# Incremental Native Shell (Shell bản địa tăng trưởng)


### 1. Giai đoạn 1: Built-in Commands (Tier 1 Cell)

Mày đừng coi Shell là một file ELF nằm trên đĩa vội. Hãy để nó là một **Tier 1 Native Cell** được nạp thẳng vào SAS.

* **Lệnh cơ bản**: Chỉ cần làm 4 lệnh: `help`, `ls`, `cat`, và `clear`.
* **Tại sao**:
* `ls` và `cat` sẽ kiểm tra xem tầng **VFS Dispatcher** và **viRamDisk** có thực sự thông chưa.
* Nó giúp mày thực hiện đúng **Luật 8 (Borrowing)**: Shell khi nhận dữ liệu từ `cat` không cần copy vào buffer riêng, mà chỉ cần mượn (`borrow`) trực tiếp từ Page Cache của VFS trong SAS.


### 2. Giai đoạn 2: Syscall Stress Test

Biến Shell thành công cụ "hành hạ" Trap Handler mày vừa viết.

* **Tận dụng `ecall**`: Mọi lệnh trong Shell phải đi qua Syscall. Ví dụ: Thay vì gọi hàm `print` của Kernel, Shell phải gọi `sys_log` để kiểm tra xem `stvec` và `sepc` có hoạt động chuẩn không.
* **Async Shell**: Áp dụng cái `ViAsyncFileSystem` mày vừa tút lại. Shell sẽ không bị treo (block) khi đợi đọc RAM Disk, giúp mày test luôn con **Async Executor** sau này.


### 3. Giai đoạn 3: External ELF Execution (The Real Test)

Khi cái Shell nội bộ đã ổn, lúc đó mới làm tính năng nạp file ELF từ `/bin/`.

* **Sức mạnh của `Vi` prefix**: Lúc này Shell sẽ gọi `ViVmRuntime` để yêu cầu Kernel tạo một không gian thực thi mới cho ứng dụng.
* **Zero-copy Loading**: Nhờ SAS, Shell chỉ cần gửi đường dẫn file, Kernel (Loader) sẽ ánh xạ (map) trực tiếp file đó từ RAM Disk vào vùng thực thi mà không cần copy dữ liệu.


### 🛠️ Chiến thuật cho dev và Agent (Áp dụng các Luật)

1. **Luật 5 (No `mod.rs`)**: Shell phải nằm trong `cells\apps\shell`.
2. **Luật 6 (Naming)**: Mọi interface giao tiếp với Kernel phải dùng tiền tố `Vi` (ví dụ: `ViSyscall`).
3. **Luật 8 (Resource)**: Cấm dùng `String` hay `Vec` vô tội vạ cho việc parse lệnh. Hãy dùng `&str` và split bằng iterator để giảm áp lực cho Heap của robot nano.
4. **Luật 8 (Borrowing)**: Khi Shell thực hiện lệnh cat, nó sẽ nhận về một Box<[u8]> từ VFS. Theo Luật 8, Shell nên mượn (borrow) dữ liệu này để in ra UART thay vì copy sang một buffer trung gian, tận dụng tối đa lợi thế của Single Address Space (SAS).


### Luồng chạy của lệnh `ls` trong Shell (Kết nối 3 khu vực)

Khi mày gõ lệnh `ls` trong `cells/apps/shell`, đây là cách 3 khu vực này phối hợp mà không cần `memcpy`:

1. **App Cell (`shell`)**: Gọi syscall `open("/")` thông qua **`libs/api`**.
2. **Service Cell (`vfs_dispatcher`)**: Nhận trap, kiểm tra bảng mount, thấy `/` thuộc về `viRamDisk`, liền gọi sang Driver tương ứng.
3. **Driver Cell (`viRamDisk`)**: Nhờ **SAS**, nó chỉ việc trả về một vùng nhớ chứa danh sách file.
4. **Kết quả**: `shell` nhận được dữ liệu và in ra UART (cũng là một Driver).


### Bộ Dispatcher "Zero-copy" (Tuân thủ Luật 8)

Điểm mấu chốt ở đây là **KHÔNG** tạo thêm `String` hay `Vec<String>`. Chúng ta sẽ "mượn" trực tiếp từ buffer nhập vào.

```rust
// user/src/shell.rs
use crate::api::ViResult; // Luật 6

pub struct ViShell<'a> {
    prompt: &'a str, // Luật 8: Dùng lifetime mượn thay vì sở hữu
}

impl<'a> ViShell<'a> {
    pub fn dispatch(&self, line: &'a str) -> ViResult<()> {
        // Luật 8: Sử dụng Iterator và &str slice để parse, không cấp phát Heap
        let mut parts = line.trim().split_whitespace();
        let cmd = parts.next().ok_or(ViHalError::InvalidInput)?;

        match cmd {
            "ls" => self.cmd_ls(parts),
            "cat" => self.cmd_cat(parts),
            "help" => self.cmd_help(),
            _ => {
                println!("ViCell: command not found: {}", cmd);
                Ok(())
            }
        }
    }
}
```


### Tận dụng VFS qua Syscall (Tuân thủ Luật 7)

Mày hãy nhìn cách lệnh `ls` tận dụng cái **Trait Object** mày vừa xây dựng:

```rust
// user/src/shell/commands.rs
impl<'a> ViShell<'a> {
    fn cmd_ls(&self, mut args: core::str::SplitWhitespace<'a>) -> ViResult<()> {
        let path = args.next().unwrap_or("/");
        
        // Gọi Syscall mở thư mục - trả về một Trait Object
        // Luật 7: Box<dyn ViFile + Send + Sync>
        let dir = crate::fs::open(path)?; 
        
        // Giả sử viFS1 hoặc viFS2 trả về danh sách entry qua SAS
        for entry in dir.read_dir()? {
            println!("{}", entry.name());
        }
        Ok(())
    }
}
```


### Tại sao cấu trúc này lại "đáng đồng tiền bát gạo"?

1. **Hiệu suất Robot Nano**: Nhờ **Luật 8 (Borrowing)**, việc mày parse một câu lệnh dài dằng dặc cũng không tốn thêm một byte nào trên Global Heap. Điều này cực kỳ quan trọng khi ViCell chạy trên các thiết bị có RAM giới hạn.
2. **Tính linh hoạt của Cell**: Vì mày dùng **Luật 7 (Trait Objects)**, cái Shell này không cần biết nó đang `ls` trên `viFS1` (RAM Disk) hay `viFS2` (TFS). Khi mày tháo RAM Disk ra và cắm một ổ cứng thật dùng `viFS2`, cái Shell vẫn chạy mà không cần sửa một dòng code nào.
3. **An toàn tuyệt đối (LBI)**: Nếu Shell bị lỗi (ví dụ: parse sai địa chỉ), cơ chế **Panic Recovery** của ViCell sẽ chỉ làm sập cái Shell Cell đó. Kernel bắt được trap, hồi sinh Shell, và mày lại thấy dấu nhắc lệnh `ViCell >` mà không phải reboot cả hệ thống.


### 🏁 Lời khuyên "Chốt hạ":
**"Code từ từ nhưng phải Native"**.

* **Bước 1**: Làm một cái vòng lặp `loop { print!("ViCell > "); input(); dispatch(); }` đơn giản nhất.
* **Bước 2**: Thực thi các lệnh bằng cách gọi trực tiếp vào `ViFileSystem` trait qua syscall.

Đừng copy `Busybox` hay `Rush` ngay lúc này, vì mày sẽ mất cả tuần để sửa lỗi tương thích. Tự code một cái Shell tí hon theo đúng "vibe" của ViCell sẽ giúp mày hiểu sâu hơn về cái "long mạch" Trap/Syscall mày vừa khai thông.