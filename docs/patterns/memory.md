# Memory Management Patterns
> Part of [Cellos Patterns](../patterns.md)

## Owned Buffers for Async (SAS Law)

**Intent**: Avoid lifetime violations in async code across Cell boundaries.

**Rule**: Never pass `&mut [u8]` across `await` points in SAS.

```rust
// ❌ Bad — lifetime violation if Cell unloads during await
async fn process(data: &mut [u8]) -> ViResult<()> {
    some_async_operation().await;  // data reference may dangle here
    data[0] = 42;
    Ok(())
}

// ✅ Good — ownership transferred, safe across await
async fn process(mut data: Box<[u8]>) -> ViResult<Box<[u8]>> {
    some_async_operation().await;  // data ownership preserved
    data[0] = 42;
    Ok(data)
}
```

**Why**: In SAS, raw pointers remain *physically* valid, but ownership rules are what protect against
logical use-after-free when a Cell unloads while another Cell holds a borrow.

## Arc for Shared Resources

**Intent**: Share immutable data across tasks without copying.

```rust
static FILESYSTEM: Spinlock<Option<Arc<dyn ViFileSystem + Send + Sync>>>
    = Spinlock::new(None);

pub fn register_filesystem(fs: impl ViFileSystem + 'static) {
    *FILESYSTEM.lock() = Some(Arc::new(fs));
}

pub fn get_filesystem() -> Option<Arc<dyn ViFileSystem + Send + Sync>> {
    FILESYSTEM.lock().as_ref().map(Arc::clone)
}
```

**When to Use**: data accessed by multiple tasks, expensive to copy, immutable or needs interior mutability.

## RAII for Resource Cleanup

**Intent**: Automatically release resources when they go out of scope.
No process cleanup in SAS — resources must clean up explicitly via Drop.

```rust
pub struct FileHandle { file: Box<dyn ViFile + Send + Sync> }

impl Drop for FileHandle {
    fn drop(&mut self) {
        // Cleanup happens automatically — no explicit close() needed
    }
}

// Lease with auto-revoke (Phase 27 — planned, learn from Hubris):
pub struct Lease { id: usize }

impl Drop for Lease {
    fn drop(&mut self) {
        kernel::ipc::revoke_lease(self.id);  // expires when call-return completes
    }
}
```

## Per-Cell Memory Quota (Phase 26 — Planned)
> Learn from: [Tock OS grant mechanism](https://github.com/tock/tock) `kernel/src/grant.rs`

```rust
// GlobalAlloc identifies caller via Program Counter range → applies quota
// Returns Err(OutOfMemory) for that cell only — not system panic
pub struct MemoryQuota {
    cell_id: CellId,
    limit_bytes: usize,
    used_bytes: AtomicUsize,
}
```

Tock pattern: kernel allocates memory for capsule *inside that process's heap*.
Capsule cannot access memory of other processes even within SAS.
