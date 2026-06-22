# Cellos on VisionFive2 — Board Bring-Up Guide

Boot Cellos on a real StarFive VisionFive2 (JH7110, RV64GC) and reach an interactive `Cellos>` shell via UART serial. No SD card driver required — all cells are embedded in the kernel binary.

---

## Hardware Required

| Item | Notes |
|------|-------|
| StarFive VisionFive2 (v1.3A or v1.3B) | 2 GB or 4 GB RAM |
| microSD card, ≥ 1 GB | Class 10 recommended |
| USB-to-UART adapter (3.3 V TTL) | CP2102, CH340G, or FTDI FT232R |
| Linux host machine | For flashing (`dd`); WSL2 works with caveats |

### UART Connection (40-pin header)

```
Pin 6  — GND
Pin 8  — TX  (board → host RX)
Pin 10 — RX  (board → host TX)
```

Host terminal settings: **115200 8N1**

```bash
minicom -D /dev/ttyUSB0 -b 115200
# or
screen /dev/ttyUSB0 115200
```

---

## Prerequisites

```bash
# Rust RISC-V target
rustup target add riscv64gc-unknown-none-elf

# Image-creation tools (Debian/Ubuntu)
sudo apt install parted util-linux dosfstools curl
```

---

## Build and Flash

```bash
# 1. Build kernel + create vf2-boot.img (256 MB GPT + FAT32)
./scripts/vf2-flash.sh

# 2. Flash to SD card (replace /dev/sdX with your device — verify with lsblk!)
sudo ./scripts/vf2-flash.sh /dev/sdX

# Windows (PowerShell + WSL2):
.\scripts\vf2-build.ps1
```

The script:
1. Downloads `BOOTRISCV64.EFI` (Limine v12) via `scripts/download-limine.sh` if not cached
2. Compiles `Cellos-kernel` with `--features board-vf2 --release`
3. Creates a 256 MB GPT image with a 200 MB EFI System Partition
4. Populates: `EFI/BOOT/BOOTRISCV64.EFI`, `limine.conf` (KASLR=no), `Cellos-kernel`

---

## Boot Sequence

Insert SD card, connect UART adapter, power on. Expected serial output:

```
[U-Boot SPL]
U-Boot SPL 2024.x (VisionFive2)
...

[Limine]
Limine 12.x.x
Loading /Cellos-kernel ...
Booting ...

[Cellos kernel]
Cellos v0.2.0 — RISC-V 64 — Cellular SAS
[boot] Limine memory map: N entries
[boot] Usable RAM: 0x44200000 – 0xCC000000
[hal] NS16550 UART @ 0x10000000 — ready
[fs] VIFS1 embedded ramdisk: 8 cells
[init] Starting Cellos Orchestrator...
[init] cell not found — skipping:
[init] /bin/compositor
[init] cell not found — skipping:
[init] /bin/input
[init] cell not found — skipping:
[init] /bin/net
[init] cell not found — skipping:
[init] /bin/robot-demo
[init] services spawned.
[init] service registry verified.
[vfs] RamFS ready
[shell] Cellos shell ready

Cellos>
```

> **Note**: "cell not found — skipping" messages are **expected** on VF2. VirtIO net/input/compositor are not present on real hardware; those cells are simply absent from the embedded ramdisk. Shell, VFS, and config still start normally.

---

## Known Limitations (Embedded Demo)

| Feature | Status | Notes |
|---------|--------|-------|
| Interactive shell (`cat`, `ls`, `echo`) | ✅ | Fully functional |
| VFS RamFS | ✅ | Embedded in kernel |
| FAT32 from microSD | ❌ | No JH7110 SD driver yet |
| Network (MQTT, TCP) | ❌ | VirtIO net absent; JH7110 Ethernet future work |
| Keyboard / HID input | ❌ | VirtIO input absent; USB HID future work |
| GPIO / robot demo | ❌ | JH7110 GPIO driver future work |

---

## Troubleshooting

| Symptom | Likely Cause | Fix |
|---------|-------------|-----|
| No serial output at all | UART TX/RX swapped | Swap the TX↔RX wires on adapter |
| No serial output | U-Boot not reading SD card | Re-flash; verify partition table: `parted -l vf2-boot.img` |
| Limine: `BOOTRISCV64.EFI not found` | Script failed silently | Re-run `vf2-flash.sh`; check `tools/limine-riscv64` exists |
| Kernel panic at `[init]` | VFS binary missing | Check `kernel/src/embedded*/` contains `vfs` ELF |
| Stuck after `[boot] Limine memory map` | Wrong DRAM base in fallback | Confirm `--features board-vf2` was passed to cargo |
| U-Boot drops to shell | U-Boot UEFI support disabled | Upgrade VF2 firmware to ≥ 3.9.x; or boot with: `load mmc 0:1 0x84000000 Cellos-kernel; bootefi 0x84000000` |
| `losetup --partscan` fails (WSL2) | Kernel loop module restricted | Run `vf2-flash.sh` from native Linux instead |

### Manual U-Boot Boot (if Limine EFI fails)

At the U-Boot `=>` prompt:

```
# Load kernel directly via U-Boot EFI stub
load mmc 0:1 0x84000000 Cellos-kernel
bootefi 0x84000000
```

---

## Hardware Compatibility Notes

The VisionFive2 JH7110 SoC uses the same PLIC (`0x0C000000`), UART NS16550 (`0x10000000`), and CLINT (`0x02000000`) base addresses as the QEMU RISC-V virt machine. No peripheral address changes were needed in Cellos to support this board.

The only hardware-specific change is the fallback DRAM base (`0x4000_0000` on JH7110 vs `0x8000_0000` on QEMU virt), active only when Limine is absent. Under normal Limine UEFI boot, the firmware provides the correct memory map automatically.
