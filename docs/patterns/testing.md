# Testing Patterns
> Part of [Cellos Patterns](../patterns.md)

## Unit Tests with Mocks

**Intent**: Test kernel code in isolation without QEMU.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct MockAllocator { next_addr: PAddr }

    impl MockAllocator {
        fn allocate(&mut self) -> Option<PAddr> {
            let addr = self.next_addr;
            self.next_addr = PAddr(self.next_addr.0 + PAGE_SIZE);
            Some(addr)
        }
    }

    #[test]
    fn test_stack_allocation() {
        let mut allocator = MockAllocator { next_addr: PAddr(0x8000_0000) };
        let stack = allocate_stack(&mut allocator).unwrap();
        assert!(stack.0 >= 0x8000_0000);
    }
}
```

## Architecture Validation Tests

**Intent**: Verify API contracts and trait invariants across implementations.

```rust
// tests/architecture-validation/step1_contract_testing.rs
#[test]
fn test_filesystem_contract() {
    let fs = FatFs::new();
    // Contract: open() returns error for nonexistent file
    assert!(fs.open("/nonexistent.txt", OpenMode::Read).is_err());
    // Contract: mkdir() creates directory
    fs.mkdir("/test_dir").unwrap();
    assert!(fs.open("/test_dir", OpenMode::Read).unwrap().is_dir());
}
```

## Integration Tests (QEMU-driven)

**Pattern**: `QemuRunner` harness boots kernel, sends shell commands, asserts on serial output.

```rust
// tests/integration/tests/boot.rs
#[test]
#[ignore]  // requires QEMU — skip in unit CI
fn shell_run_echo() {
    let mut qemu = QemuRunner::new().expect("QEMU not available");
    qemu.wait_for("$ ", BOOT_TIMEOUT).expect("shell prompt");
    qemu.send_command("echo hello\n");
    qemu.wait_for("hello", CMD_TIMEOUT).expect("echo output");
}
```

**Key constants**: `BOOT_TIMEOUT = 40s`, `CMD_TIMEOUT = 10s`. Tests skip (not fail) if QEMU or disk image missing.

## Fault Injection (Planned)
> Per spec 10-testing.md — Chaos Engineering Cell

- Random Cell panics → verify other Cells continue
- 99% RAM allocation → verify OOM kills only that Cell
- Deadlock simulation → verify Deadlock Watchdog fires
