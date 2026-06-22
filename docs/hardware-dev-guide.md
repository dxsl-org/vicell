# Hardware Development Guide — Real Board Workflow

> **Version**: 0.1 | **Last Updated**: 2026-06-07
>
> Hướng dẫn phát triển, kiểm thử, và gỡ lỗi Cellos trên board phần cứng thật.
> Đọc [getting-started.md](./getting-started.md) trước nếu chưa build/chạy được trên QEMU.

---

## 1. Tổng quan chiến lược: QEMU-first, Board-validate

Cellos áp dụng nguyên tắc **QEMU-first** (xem [specs/04-hardware.md §7](./specs/04-hardware.md)):
mọi HAL trait và driver cell được phát triển & kiểm thử trên QEMU trước,
board thật chỉ dùng để **validate** những gì QEMU không thể emulate.

| Công việc | QEMU | Board thật |
|---|:---:|:---:|
| Kernel logic, scheduler, IPC, VFS | ✅ | — |
| HAL trait interface design | ✅ | — |
| GPIO/UART driver cells (PL061/PL011) | ✅ | ✅ validate `impl` trên silicon |
| I2C/SPI sensor drivers | ❌ | ✅ **bắt buộc** |
| RT latency benchmark | ❌ (không chính xác) | ✅ **bắt buộc** |
| Power management | ❌ | ✅ **bắt buộc** |
| Security audit / fuzzing | ✅ | — |

**Nguyên tắc**: Code trên QEMU trước. Khi pass → deploy board thật chỉ để validate
hardware-specific behavior. Nếu phát hiện lỗi trên board → quay lại QEMU reproduce nếu
có thể, sửa, rồi mới deploy lại.

---

## 2. Target boards

### G1 boards (mua trước)

| Board | CPU | Arch | Giá ước tính | Ưu tiên |
|---|---|---|---|---|
| **Raspberry Pi 4 Model B** | BCM2711 (Cortex-A72) | ARM64 | ~1.5M VNĐ | ⭐ Mua đầu tiên |
| **VisionFive 2** | JH7110 (SiFive U74) | RV64 | ~2M VNĐ | Mua khi ARM64 stable |

**Tại sao RPi 4 trước?**
- Community lớn nhất → dễ tìm giải pháp khi gặp lỗi boot
- PL011 UART giống hệt QEMU ARM virt → UART driver cell không cần thay đổi
- Nhiều tài liệu bare-metal (bare-metal RPi4 tutorial, `raspberrypi/firmware` repo)
- JTAG debug tương đối dễ setup

### G2/G3 boards (mua sau)

| Board | Arch | Mục đích |
|---|---|---|
| Milk-V Pioneer (SG2042) | RV64 | Server-class RV64, PCIe test |
| Radxa ROCK 5B (RK3588) | ARM64 | NPU driver (RKNN) |
| Sonata (CHERIoT) | RV32 | Cellos-Nano sub-track |

---

## 3. Thiết bị cần mua

### Bắt buộc

| Thiết bị | Mục đích | Giá ước tính |
|---|---|---|
| **USB-to-UART adapter** (CP2102 hoặc FT232RL) | Serial console — xem kernel log, dùng shell | ~50K VNĐ |
| **MicroSD card** (≥16 GB, class 10) + reader | Flash U-Boot + kernel image | ~100K VNĐ |
| **Ethernet cable** (Cat5e+) | TFTP deploy — vòng lặp dev nhanh | ~30K VNĐ |
| **USB-C power** (5V 3A, RPi4) | Cấp nguồn | ~100K VNĐ |

### Khuyến nghị (debug nâng cao)

| Thiết bị | Mục đích | Giá ước tính |
|---|---|---|
| **FT2232H mini module** (JTAG adapter) | Hardware breakpoint, step-debug trước MMU init | ~300K VNĐ |
| **Logic analyzer** (8 channel, 24 MHz) | Debug SPI/I2C timing, GPIO waveforms | ~200K VNĐ |
| **Breadboard + jumper wires** | Kết nối cảm biến test | ~50K VNĐ |
| **BME280 / MPU6050 breakout** | I2C sensor — test driver cell đầu tiên | ~50-80K VNĐ |

### Sơ đồ kết nối vật lý (RPi 4)

```
┌─────────────────────────────────────────────────┐
│                Raspberry Pi 4                    │
│                                                  │
│  GPIO 14 (TX) ──→  RX ┐                        │
│  GPIO 15 (RX) ←──  TX ├── USB-UART ──→ PC USB  │
│  GND          ──→  GND┘   (CP2102)             │
│                                                  │
│  GPIO 2 (SDA) ──→  SDA ┐                       │
│  GPIO 3 (SCL) ──→  SCL ├── BME280 sensor       │
│  3.3V         ──→  VCC │                        │
│  GND          ──→  GND ┘                        │
│                                                  │
│  Ethernet     ──────────────→ PC / Router        │
│  USB-C        ──────────────→ 5V 3A PSU         │
│                                                  │
│  (Optional JTAG via FT2232H:)                   │
│  GPIO 22 (TCK) ──→ TCK ┐                       │
│  GPIO 27 (TMS) ──→ TMS ├── FT2232H ──→ PC USB  │
│  GPIO 25 (TDI) ──→ TDI │                        │
│  GPIO 24 (TDO) ←── TDO │                        │
│  GND           ──→ GND ┘                        │
└─────────────────────────────────────────────────┘
```

---

## 4. Chuẩn bị SD card (one-time setup)

Board thật cần bootloader (U-Boot) trên SD card. Kernel Cellos sẽ được U-Boot nạp.

### RPi 4

```bash
# 1. Download RPi firmware + U-Boot for RPi4
git clone --depth 1 https://github.com/raspberrypi/firmware rpi-firmware

# 2. Format SD card: 1 partition, FAT32
# Windows: dùng Disk Management hoặc Rufus
# Linux:   mkfs.vfat /dev/sdX1

# 3. Copy boot files to SD
cp rpi-firmware/boot/bootcode.bin  /SD/
cp rpi-firmware/boot/start4.elf   /SD/
cp rpi-firmware/boot/fixup4.dat   /SD/
cp u-boot.bin                     /SD/kernel8.img  # U-Boot thay thế Linux kernel

# 4. Tạo config.txt
cat > /SD/config.txt << 'EOF'
arm_64bit=1
enable_uart=1
kernel=kernel8.img
# JTAG debug (bỏ comment khi cần):
# enable_jtag_gpio=1
# gpio=22-27=a4
EOF
```

### VisionFive 2

```bash
# VisionFive 2 có U-Boot trên SPI flash sẵn.
# Chỉ cần SD card FAT32 chứa kernel image.
# U-Boot sẽ tự tìm và load từ SD hoặc TFTP.

# Format SD, copy kernel:
cp Cellos-kernel.img /SD/Cellos.bin
```

---

## 5. Deploy kernel — 3 phương pháp

### Phương pháp A: SD card (chậm nhất — dùng khi lần đầu hoặc không có mạng)

```powershell
# Build
$env:RUSTFLAGS = "-C relocation-model=pic"
cargo build --release -p Cellos-kernel --target aarch64-unknown-none-softfloat
$env:RUSTFLAGS = $null

# Copy kernel lên SD card (giả sử ổ E:)
Copy-Item target/aarch64-unknown-none-softfloat/release/Cellos-kernel E:\Cellos.bin

# Rút SD → cắm board → bật nguồn → xem serial output
```

**Iteration time**: ~2 phút (rút SD, copy, cắm lại, reboot).

### Phương pháp B: U-Boot + TFTP (recommended cho daily dev)

TFTP cho phép deploy kernel qua mạng Ethernet mà **không cần rút SD card**.

```
┌─── PC Dev ──────────────────┐          ┌─── Board ──────────────┐
│                              │          │                        │
│  1. cargo build              │          │  U-Boot console        │
│  2. Copy kernel → tftp/      │   ETH    │  (qua serial)          │
│  3. TFTP server listening ──────────────→  tftp → boot kernel    │
│  4. Serial monitor ←─────────── USB ────←  UART output           │
│                              │          │                        │
└──────────────────────────────┘          └────────────────────────┘
```

#### Setup TFTP server trên PC

**Windows** — dùng [Tftpd64](https://pjo2.github.io/tftpd64/):
1. Download Tftpd64, chạy portable (không cần cài)
2. Settings → TFTP → Base Directory: `D:\Cellos\tftp`
3. Bind IP: IP Ethernet của PC (ví dụ: `192.168.1.100`)

**Linux/macOS**:
```bash
sudo apt install tftpd-hpa
# hoặc: brew install tftp-hpa
# Config: /etc/default/tftpd-hpa → TFTP_DIRECTORY="/home/user/Cellos/tftp"
sudo systemctl restart tftpd-hpa
```

#### Deploy script

```powershell
# scripts/deploy-rpi4.ps1

param(
    [string]$TftpDir = "D:\Cellos\tftp",
    [string]$Target  = "aarch64-unknown-none-softfloat"
)

$kernel = "target/$Target/release/Cellos-kernel"

Write-Host "Building aarch64 release kernel..."
$env:RUSTFLAGS = "-C relocation-model=pic"
cargo build --release -p Cellos-kernel --target $Target
$env:RUSTFLAGS = $null

if (-not (Test-Path $kernel)) {
    Write-Host "Build failed." -ForegroundColor Red
    exit 1
}

# Đảm bảo TFTP directory tồn tại
New-Item -ItemType Directory -Force -Path $TftpDir | Out-Null

Copy-Item $kernel "$TftpDir\Cellos.bin" -Force
$size = (Get-Item "$TftpDir\Cellos.bin").Length / 1MB
Write-Host "Deployed Cellos.bin ($([math]::Round($size, 1)) MB) to TFTP." -ForegroundColor Green
Write-Host ""
Write-Host "On U-Boot serial console, run:" -ForegroundColor Yellow
Write-Host "  setenv serverip 192.168.1.100"
Write-Host "  setenv ipaddr   192.168.1.200"
Write-Host "  tftp 0x40000000 Cellos.bin"
Write-Host "  go   0x40000000"
```

#### U-Boot auto-boot (không cần gõ lệnh mỗi lần)

Thêm vào `boot.scr` trên SD card để U-Boot tự TFTP và boot:

```bash
# boot.cmd (compile bằng mkimage)
setenv serverip 192.168.1.100
setenv ipaddr   192.168.1.200
tftp 0x40000000 Cellos.bin
go 0x40000000
```

```bash
# Compile thành boot.scr
mkimage -C none -A arm64 -T script -d boot.cmd boot.scr
# Copy boot.scr lên SD card
```

Khi có `boot.scr`, workflow trở thành: **build → copy TFTP → nhấn reset trên board** → tự
động boot Cellos mới nhất. **Iteration time: ~15 giây**.

### Phương pháp C: UART xmodem (backup — chỉ cần 1 dây USB)

Khi không có Ethernet (ví dụ test ngoài bàn lab):

```
# U-Boot console:
u-boot> loady 0x40000000 115200    # chờ nhận file qua UART YMODEM

# PC: dùng TeraTerm → File → Transfer → YMODEM → Send → chọn Cellos.bin
# Sau khi truyền xong:
u-boot> go 0x40000000
```

**Iteration time**: ~3-5 phút (UART chậm, ~11 KB/s ở 115200 baud). Chỉ dùng khi không
có lựa chọn khác.

---

## 6. Serial monitor (bắt buộc)

Serial monitor là **cửa sổ duy nhất** để tương tác với Cellos trên board thật (tương đương
`-nographic` trên QEMU). Kernel log, shell prompt, panic message đều xuất ra đây.

### Tìm COM port

```powershell
# Windows: mở Device Manager → Ports (COM & LPT) → tìm "USB-SERIAL" → ghi nhớ COM number
# Hoặc PowerShell:
Get-WMIObject Win32_SerialPort | Select-Object Name, DeviceID
```

### Mở serial

**Cách 1 — PuTTY** (Windows, GUI):
- Connection type: Serial
- Serial line: `COM3` (thay đổi theo máy)
- Speed: `115200`

**Cách 2 — Windows Terminal + plink**:
```powershell
plink -serial COM3 -sercfg 115200,8,n,1,N
```

**Cách 3 — minicom** (Linux/macOS/WSL2):
```bash
sudo minicom -b 115200 -D /dev/ttyUSB0
# hoặc /dev/ttyACM0 tuỳ adapter
```

**Cách 4 — Script tự động**:

```powershell
# scripts/serial-monitor.ps1
param(
    [string]$Port = "COM3",
    [int]$Baud = 115200
)

$portObj = New-Object System.IO.Ports.SerialPort $Port, $Baud, None, 8, One
$portObj.ReadTimeout = 500

try {
    $portObj.Open()
    Write-Host "Connected to $Port at $Baud baud. Ctrl+C to exit." -ForegroundColor Green
    while ($true) {
        try {
            $line = $portObj.ReadLine()
            Write-Host $line
        } catch [System.TimeoutException] { }

        if ([Console]::KeyAvailable) {
            $key = [Console]::ReadKey($true)
            $portObj.Write($key.KeyChar.ToString())
        }
    }
} finally {
    $portObj.Close()
}
```

### Cấu hình serial phổ biến

| Board | UART | Baud | GPIO pins |
|---|---|---|---|
| RPi 4 | PL011 (UART0) | 115200 | TX=GPIO14, RX=GPIO15 |
| VisionFive 2 | UART0 | 115200 | TX=pin 6, RX=pin 8 (40-pin header) |

---

## 7. Debug trên board thật

### Level 1: Serial log (luôn dùng)

Giống QEMU — kernel `log::info!()` / `log::error!()` tự xuất ra UART.
Đây là phương pháp debug chính và đơn giản nhất.

```rust
// kernel/src/main.rs hoặc bất kỳ đâu
log::info!("Frame allocator: {} frames free", free_count);
log::error!("GPIO init failed: {:?}", err);
```

Mẹo:
- Dùng `log::trace!()` cho thông tin chi tiết, filter bằng log level
- In hex dump cho raw register values: `log::debug!("GPFSEL0 = {:#010x}", val);`

### Level 2: GDB remote debug qua JTAG

JTAG cho phép **hardware breakpoints** — hoạt động ngay cả khi kernel hang, trước
khi MMU được init, hoặc trong interrupt handler. **Đây là cách debug mạnh nhất.**

#### Setup

```
┌─── PC ───────────────────┐          ┌─── Board ─────────┐
│                           │          │                    │
│  OpenOCD ←─── USB ──────────────────→ FT2232H ──→ JTAG  │
│    ↕                      │          │  pins trên board   │
│  GDB ←── tcp:3333         │          │                    │
│                           │          │                    │
└───────────────────────────┘          └────────────────────┘
```

#### Cài đặt OpenOCD

```powershell
# Windows: tải binary từ https://github.com/openocd-org/openocd/releases
# Linux:
sudo apt install openocd
# macOS:
brew install openocd
```

#### Chạy debug session

```powershell
# Terminal 1: OpenOCD kết nối JTAG → board
# RPi 4:
openocd -f interface/ftdi/ft2232h-module-swd.cfg -f target/bcm2711.cfg

# VisionFive 2:
openocd -f interface/ftdi/ft2232h-module-swd.cfg -f target/sifive-u74.cfg
```

```powershell
# Terminal 2: GDB kết nối tới OpenOCD
# ARM64:
aarch64-none-elf-gdb target/aarch64-unknown-none-softfloat/debug/Cellos-kernel

# RISC-V:
riscv64-unknown-elf-gdb target/riscv64gc-unknown-none-elf/debug/Cellos-kernel

# Trong GDB:
(gdb) target remote localhost:3333
(gdb) monitor reset halt          # reset board, dừng ở instruction đầu tiên
(gdb) break kmain                 # breakpoint tại kernel main
(gdb) continue                    # chạy đến breakpoint
(gdb) info registers              # xem thanh ghi
(gdb) x/16xw 0x09030000          # dump GPIO registers (PL061)
(gdb) print *scheduler            # inspect scheduler state
(gdb) backtrace                   # call stack
```

> **Lưu ý**: GDB workflow **gần như giống hệt** QEMU (`-s -S` + `target remote localhost:1234`).
> Chỉ khác port (3333 thay vì 1234) và cần OpenOCD thay vì QEMU GDB stub.

#### GDB script tự động

```powershell
# scripts/debug-board.ps1

param(
    [ValidateSet("rpi4", "vf2")]
    [string]$Board = "rpi4",
    [string]$BuildProfile = "debug"
)

$configs = @{
    "rpi4" = @{
        OpenOcdTarget = "target/bcm2711.cfg"
        GdbExe        = "aarch64-none-elf-gdb"
        Target        = "aarch64-unknown-none-softfloat"
    }
    "vf2" = @{
        OpenOcdTarget = "target/sifive-u74.cfg"
        GdbExe        = "riscv64-unknown-elf-gdb"
        Target        = "riscv64gc-unknown-none-elf"
    }
}

$cfg = $configs[$Board]
$kernel = "target/$($cfg.Target)/$BuildProfile/Cellos-kernel"

Write-Host "Starting OpenOCD for $Board..." -ForegroundColor Cyan
$openocd = Start-Process -FilePath "openocd" `
    -ArgumentList "-f interface/ftdi/ft2232h-module-swd.cfg -f $($cfg.OpenOcdTarget)" `
    -PassThru -NoNewWindow

Start-Sleep -Seconds 2

Write-Host "Connecting GDB to $kernel..." -ForegroundColor Cyan
& $cfg.GdbExe $kernel `
    -ex "target remote localhost:3333" `
    -ex "monitor reset halt" `
    -ex "break kmain" `
    -ex "continue"

Stop-Process $openocd
```

### Level 3: Logic analyzer (cho driver debug)

Khi debug I2C/SPI driver cells, serial log không đủ — cần xem tín hiệu thực tế trên
dây. Dùng logic analyzer rẻ tiền (Saleae clone, ~200K VNĐ):

```
Logic analyzer probes:
  CH0 ──→ SCL (I2C clock)
  CH1 ──→ SDA (I2C data)
  CH2 ──→ GPIO output (để đo timing)
  GND ──→ board GND
```

Phần mềm: [PulseView](https://sigrok.org/wiki/PulseView) (miễn phí, hỗ trợ I2C/SPI
protocol decode).

---

## 8. Testing trên board thật

### Unit tests vs Integration tests

| Loại | Chạy đâu | Cách chạy |
|---|---|---|
| Unit tests (logic thuần) | PC host | `cargo test --lib` (không cần board) |
| HAL integration tests | QEMU trước, board sau | `cargo test --target aarch64-...` + QEMU |
| **Peripheral integration** | **Board thật** | Deploy kernel có test harness |
| **RT latency benchmark** | **Board thật** | `cells/apps/bench` + timer đo |

### Peripheral integration test workflow

Tạo build mode đặc biệt cho test trên board thật:

```rust
// cells/apps/periph-test/src/main.rs
// Tự động test GPIO, UART, I2C khi boot — in kết quả ra serial

#![forbid(unsafe_code)]

fn main() {
    ostd::println!("=== Cellos Peripheral Test Suite ===");

    // GPIO test: toggle pin, read back
    ostd::println!("[TEST] GPIO toggle...");
    let gpio = ostd::gpio::open(0).expect("GPIO open failed");
    gpio.set_output(5);
    gpio.write(5, true);
    assert!(gpio.read(5), "GPIO readback failed");
    ostd::println!("[PASS] GPIO toggle");

    // UART loopback test (TX→RX shorted)
    ostd::println!("[TEST] UART loopback...");
    // ... test code ...

    ostd::println!("=== ALL TESTS PASSED ===");
}
```

### RT latency benchmark

```rust
// cells/apps/bench/src/rt_latency.rs
// Đo interrupt latency thực tế trên silicon

fn measure_irq_latency() {
    let timer = ostd::timer::high_res();

    for i in 0..1000 {
        let t0 = timer.now_ns();
        // Trigger software interrupt
        ostd::syscall::trigger_irq();
        let t1 = timer.now_ns();

        let latency_us = (t1 - t0) / 1000;
        ostd::println!("IRQ latency[{}]: {} µs", i, latency_us);
    }
    // Kết quả trên QEMU: ~50-200 µs (vô nghĩa — QEMU softemu)
    // Kết quả trên RPi4:  ~1-5 µs    (số liệu thật cho tiêu chí G1 #3)
}
```

---

## 9. Vòng lặp phát triển hàng ngày (daily workflow)

### Iteration tốc độ cao với TFTP

```
  Edit code ──→ Build (5s) ──→ TFTP copy (1s) ──→ Board reset (3s) ──→ Serial log
      ↑                                                                      │
      └──────────────────── Fix bug ←────────────────────────────────────────┘

  Tổng: ~15 giây/iteration (so với ~5s trên QEMU, ~2 phút với SD card)
```

### Recommended 2-terminal layout

```
┌──────────────────────────────┬──────────────────────────────┐
│  Terminal 1: Build & Deploy  │  Terminal 2: Serial Monitor  │
│                              │                              │
│  PS> .\scripts\deploy-       │  Connected to COM3 @ 115200  │
│      rpi4.ps1                │                              │
│  Building aarch64 kernel...  │  Cellos K MAIN ENTRY         │
│  Deployed Cellos.bin (4.2MB) │  [INFO] Kernel started       │
│                              │  [INFO] Frame alloc OK       │
│  (board tự reboot nếu có    │  Cellos>                      │
│   auto-boot script)          │                              │
│                              │                              │
└──────────────────────────────┴──────────────────────────────┘
```

### Khi nào dùng GDB thay vì serial log?

| Tình huống | Serial log | GDB/JTAG |
|---|---|---|
| Kernel boot bình thường, debug logic | ✅ | — |
| Kernel hang (không in gì ra serial) | ❌ | ✅ |
| Panic trong interrupt handler | ⚠️ có thể mất output | ✅ |
| Debug MMU/page table setup | ❌ | ✅ |
| Inspect hardware register values | ⚠️ phải thêm print code | ✅ (`x/xw addr`) |
| Bước qua code từng dòng | ❌ | ✅ (`step`, `next`) |

---

## 10. Troubleshooting phổ biến

| Triệu chứng | Nguyên nhân | Giải pháp |
|---|---|---|
| Serial không hiện gì | Sai TX/RX, sai baud rate | Đảo TX↔RX, kiểm tra 115200 baud |
| Hiện ký tự lạ (garbage) | Sai baud rate | Thử 9600, 38400, 57600, 115200 |
| U-Boot hiện nhưng kernel hang | Kernel load ở sai address | Kiểm tra `go 0x40000000` khớp linker script |
| TFTP timeout | Firewall chặn, sai IP | Tắt Windows Firewall cho tftpd64, ping kiểm tra |
| Kernel boot trên QEMU nhưng crash trên board | HAL `impl` khác nhau, timing issue | GDB debug, so sánh register state |
| GPIO không toggle | Pin mux chưa cấu hình | Kiểm tra alt-function register (GPFSEL trên RPi) |
| I2C không giao tiếp | Pull-up resistor thiếu | Thêm 4.7KΩ pull-up trên SDA và SCL |

---

## 11. Checklist: Board thật lần đầu

Khi nhận được board mới, thực hiện theo thứ tự:

- [ ] **Bước 1**: Cắm USB-UART, mở serial monitor, bật board → xác nhận thấy U-Boot console
- [ ] **Bước 2**: Setup mạng TFTP giữa PC và board (ping test)
- [ ] **Bước 3**: TFTP deploy một kernel "hello world" → xác nhận boot thành công trên serial
- [ ] **Bước 4**: Test GDB/JTAG nếu có adapter → xác nhận breakpoint hoạt động
- [ ] **Bước 5**: Chạy `periph-test` cell → xác nhận GPIO/UART driver hoạt động
- [ ] **Bước 6**: Kết nối I2C sensor → chạy driver test đầu tiên
- [ ] **Bước 7**: Chạy RT latency benchmark → ghi nhận baseline

---

## 12. So sánh QEMU vs Board — quick reference

| Tiêu chí | QEMU | Board thật |
|---|---|---|
| **Deploy time** | ~5s (instant) | ~15s (TFTP) / ~2min (SD) |
| **Debug** | GDB `-s -S` | GDB + OpenOCD + JTAG |
| **Serial** | `-nographic` (stdin/stdout) | USB-UART adapter |
| **GPIO/UART** | PL061/PL011 (emulated) | Silicon thật |
| **I2C/SPI** | ❌ Không có | ✅ |
| **Interrupt latency** | Không chính xác | Chính xác |
| **CI/CD** | ✅ Chạy trên server | ⚠️ Cần board farm |
| **Reproducibility** | ✅ Deterministic | ⚠️ Có yếu tố vật lý |

---

## Xem thêm

- [getting-started.md](./getting-started.md) — Setup build environment + QEMU
- [specs/04-hardware.md](./specs/04-hardware.md) — Multi-arch HAL strategy + target boards
- [IDEAS.md](./IDEAS.md) — Roadmap: tuần 6-10 board validation
- [system-architecture.md](./system-architecture.md) — Kiến trúc tổng thể Cellos
