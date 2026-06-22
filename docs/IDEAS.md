## App
- Port app đầu tiên cho Cellos: https://github.com/lasselian/prism-desktop

## Task


## Defer
- SerialHandle — cần service IPC mới cho serial driver, scope lớn hơn
- embedded_io_async — cần async executor integration, làm sau Phase 1-3 ổn định

### https://github.com/orgs/rust-embedded/repositories

Bước 7: riscv-peripheral (PLIC/CLINT) — cần verify trước

Nếu vẫn còn raw MMIO PLIC writes trong hal/arch/riscv/, dùng plic_codegen! macro


### SKIP
cortex-m* family — Cortex-M only, Cellos target là A-class + RISC-V
svd2rust / svdtools — Cellos không dùng SVD
linux-embedded-hal, rust-sysfs-* — Linux sysfs, không liên quan

### Reference (đọc học, không copy code)
- rust-raspberrypi-OS-tutorials — ground truth cho EL2→EL1, GIC v2, PL011 (đối chiếu hal/arch/arm/)
- awesome-embedded-rust — discovery tool tìm sensor driver crates tương thích embedded-hal
