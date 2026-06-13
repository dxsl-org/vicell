# ViCell Architecture: Testing & Verification
**Version**: 0.3 (SAS-Specific Quality Assurance)
**Status**: Definitive

---

## 1. Triết lý: Test trong "Nồi lẩu" SAS
Trong mô hình Single Address Space, một lỗi nhỏ có thể phá hủy toàn bộ hệ thống. Do đó, testing không chỉ là kiểm tra logic mà là kiểm tra ranh giới (Boundaries).

## 2. Các tầng Testing
1. **KUnit (In-Kernel Unit Tests)**:
    * Chạy trực tiếp bên trong Ring 0 (QEMU hoặc Hardware).
    * Kiểm tra các Trait thực thi của `hal/core` và các logic lõi của Nano Kernel.
2. **Cell Integration Tests**:
    * Mô phỏng việc nạp/gỡ (Load/Unload) các Cell liên tục để kiểm tra rò rỉ bộ nhớ trong `Metadata Registry`.
3. **SASan (Single Address Space Sanitizer)**:
    * Công cụ dùng lúc debug để phát hiện một Cell cố tình truy cập vào vùng nhớ của Cell khác mà không có quyền sở hữu (Ownership).

## 3. Fault Injection Cell (Kẻ phá hoại)
Một Cell đặc biệt được thiết kế để:
* Gây ra **Panic** ngẫu nhiên trong các callback để test cơ chế `catch_unwind`.
* Chiếm dụng 99% RAM để test cơ chế **Memory Quota**.
* Gây ra **Deadlock** để test bộ phận giám sát (Watchdog).

## 4. Hardware-in-the-loop (HITL)
Đặc biệt quan trọng cho Robot:
* Kiểm tra độ trễ phản hồi (Latency) từ lúc có IRQ thực tế đến khi Driver Cell nhận được `Waker`.
* Kiểm tra việc tiêu thụ năng lượng của các Task trong trạng thái **Tickless Idle**.

---

## 5. Integration Test Guide (English)

This section documents the QEMU-driven integration test harness introduced in
Phase 11 and how to add new tests.

### Test Pyramid

```
   ┌─────────────────────────────────┐
   │  Integration (QEMU-driven)      │   ~10 tests, ~3 min
   │  tests/integration/*.rs         │
   ├─────────────────────────────────┤
   │  Boot-time kernel tests         │   ~30 tests, invoked at boot
   │  kernel/src/*/tests.rs          │
   │  kernel/src/loader/elf_tests.rs │
   ├─────────────────────────────────┤
   │  Host unit tests (cargo test)   │   15+ tests, <1 sec
   │  libs/types/src/lib.rs          │
   │  libs/api/src/syscall_tests.rs  │
   └─────────────────────────────────┘
```

### Host Unit Tests

Run with an explicit host target (the workspace default is RISC-V bare-metal):

```bash
# Windows
cargo test -p types -p api --target x86_64-pc-windows-msvc

# Linux / macOS
cargo test -p types -p api --target x86_64-unknown-linux-gnu
```

`libs/types` and `libs/api` use `#![cfg_attr(not(test), no_std)]` so that the
`#[test]` harness can link against `std` on the host while production builds
remain bare-metal.

### Boot-time Kernel Tests

These tests run inside the kernel at boot (Ring 0, no MMU isolation required).
They are `pub fn run_all()` functions called from `kernel/src/main.rs`:

```rust
// kernel/src/main.rs (boot sequence)
memory::tests::run_all();
task::tests::run_scheduler_tests();
loader::elf_tests::run_all();
```

Add a new boot-time test:

1. Create `kernel/src/<module>/my_tests.rs` with a `pub fn run_all()` function
2. Add `pub mod my_tests;` to `kernel/src/<module>.rs`
3. Call `<module>::my_tests::run_all()` from `kernel/src/main.rs`

### QEMU Integration Tests

Integration tests live in `tests/integration/` and use `QemuRunner` from
`tests/integration/harness.rs` to spawn QEMU, inject serial input, and assert
on serial output.

**Canonical pattern:**

```rust
// tests/integration/my_scenario.rs
use super::harness::QemuRunner;

const KERNEL: &str = "target/riscv64gc-unknown-none-elf/release/kernel";

pub fn test_my_feature() {
    let mut q = QemuRunner::new_rv64(KERNEL);
    q.wait_for("[ViCell]", 30).expect("kernel banner not seen");
    q.wait_for("my-expected-output", 30)
        .expect("feature output not seen");
    assert!(!q.output_contains("PANIC"));
}
```

**Adding a new integration test:**

1. Create `tests/integration/<name>.rs` using the pattern above
2. Reference the harness with `use super::harness::QemuRunner;`
3. Run via a future `ViCell-tests` host-target crate (use `--target x86_64-...`)
4. Mark QEMU-dependent tests `#[ignore]` in CI until QEMU is available on the runner

**Existing integration test files:**

| File | Covers |
|------|--------|
| `tests/integration/harness.rs` | `QemuRunner` helper |
| `tests/integration/ring3_smoke.rs` | boot banner, Ring-3 hello, shell prompt, no-panic |
| `tests/integration/multi_cell.rs` | init→config→vfs→shell chain |

### Coverage Measurement

```bash
# Requires llvm-tools-preview component
rustup component add llvm-tools-preview

# Run coverage (host-runnable crates only)
cargo llvm-cov --target x86_64-unknown-linux-gnu -p types -p api --html
# Report at: target/llvm-cov/html/index.html
```

For kernel coverage, use `scripts/measure-coverage.sh` (requires QEMU and
instrumented kernel build).