# Anti-Patterns to Avoid
> Part of [ViCell Patterns](../patterns.md)

## Unsafe Code in Cells

```rust
// ❌ Missing #![forbid(unsafe_code)] — CI gate (cargo-geiger) will reject
fn main() {
    unsafe { *(0x8000_0000 as *mut u8) = 0; }  // violates LBI
}

// ✅ Enforced by compiler
#![forbid(unsafe_code)]
fn main() { ostd::println!("Hello"); }
```

**Enforcement**: `cargo-geiger` CI gate on every PR. No exceptions for Cell crates.

## mod.rs Files (ViCell Law 5)

```
❌ memory/mod.rs       (old Rust style)

✅ memory.rs           (module root)
   memory/frame.rs     (submodule)
   memory/paging.rs    (submodule)
```

## Hardcoded Pointer Sizes (ViCell Law 3)

```rust
// ❌ Breaks on 32-bit targets
const KERNEL_BASE: u64 = 0xFFFF_FFFF_8000_0000;

// ✅ Portable across RV32/RV64/AArch64
const KERNEL_BASE: VAddr = VAddr(0x8000_0000);
```

## Global Mutable State Without Synchronization

```rust
// ❌ Data race — UB in Rust
static mut COUNTER: usize = 0;
pub fn increment() { unsafe { COUNTER += 1; } }

// ✅ Spinlock-protected
static COUNTER: Spinlock<usize> = Spinlock::new(0);
pub fn increment() { *COUNTER.lock() += 1; }
```

## Borrowed Buffers for Async (ViCell Law 2)

```rust
// ❌ Lifetime violation — borrow may dangle if Cell unloads during await
async fn send(data: &[u8]) -> ViResult<()> { async_send(data).await }

// ✅ Ownership transferred — safe across await + Cell lifecycle
async fn send(data: Box<[u8]>) -> ViResult<()> { async_send(data).await }
```

## Raw IPC Byte Buffers (Phase 27 fix)

```rust
// ❌ Current: magic opcode + raw bytes — protocol mismatch only caught at runtime
let mut buf = [0u8; 512];
buf[0] = OP_READ;

// ✅ Target (Phase 27): typed enum — protocol order enforced at compile time
send_ipc(VfsRequest::Read { fd, buf_len })?;
```

## Pattern Decision Matrix

| Scenario | Pattern | Why |
|---|---|---|
| Global state | `Spinlock<Option<T>>` | Thread-safe, late init |
| Shared resource | `Arc<dyn Trait>` | Multiple owners, no copy |
| Resource cleanup | RAII + `Drop` | Automatic, panic-safe |
| Async I/O | `Box<[u8]>` ownership | No lifetime across `await` |
| Error propagation | `Result<T, ViError>` | Caller must handle |
| Invariant violation | `panic!` / `expect()` | Unrecoverable, never in Cells |
| Complex construction | Builder | Many optional params |
| Type safety | Newtype (`VAddr`, `PAddr`) | Prevent mixing |
| Capability grant | ZST token (Phase 26) | Unforgeable, zero cost |
