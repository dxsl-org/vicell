# API Design Patterns
> Part of [Cellos Patterns](../patterns.md)

## Builder Pattern

**Intent**: Construct complex objects step-by-step with optional parameters.

```rust
pub struct TaskBuilder {
    name: Option<String>,
    cell_id: CellId,
    entry_point: VAddr,
    stack_size: usize,
}

impl TaskBuilder {
    pub fn new(cell_id: CellId, entry_point: VAddr) -> Self {
        Self { name: None, cell_id, entry_point, stack_size: DEFAULT_STACK_SIZE }
    }
    pub fn name(mut self, name: String) -> Self { self.name = Some(name); self }
    pub fn stack_size(mut self, size: usize) -> Self { self.stack_size = size; self }
    pub fn build(self) -> ViResult<Task> {
        Task::new(self.cell_id, self.name.unwrap_or_default(),
                  self.entry_point, self.stack_size)
    }
}

let task = TaskBuilder::new(cell_id, entry_point)
    .name("net_task".to_string())
    .stack_size(128 * 1024)
    .build()?;
```

## Newtype Pattern

**Intent**: Type-safe wrappers that prevent mixing incompatible values.

```rust
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VAddr(pub usize);

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PAddr(pub usize);

fn map_page(vaddr: VAddr, paddr: PAddr) { ... }  // compiler rejects swapped args
```

## Facade Pattern

**Intent**: Unified interface that hides multi-arch complexity.

```rust
// hal/core/src/lib.rs — kernel uses hal:: regardless of target
#[cfg(feature = "riscv64")]  pub use hal_riscv::rv64::*;
#[cfg(feature = "aarch64")]  pub use hal_arm::aarch64::*;
#[cfg(feature = "x86_64")]   pub use hal_x86::x86_64::*;
```

## Typed IPC Channels (Phase 27 — Planned)
> Learn from: [Singularity SIP typed channels](https://www.microsoft.com/en-us/research/publication/singularity-rethinking-the-software-stack/)

Replace raw `[u8; 512]` buffers with typed request enums — compiler enforces protocol.

```rust
// libs/api/src/vfs_channel.rs (Phase 27 target)
pub enum VfsRequest {
    Open  { path: ArrayString<253> },
    Read  { fd: u32, buf_len: u32 },
    Write { fd: u32, data: Box<[u8]> },
    Close { fd: u32 },
}
// Compiler rejects Read before Open — protocol order enforced at type level
```

## ZST Capability Tokens (Phase 26 — Planned)
> Learn from: [Midori capabilities-as-types](https://joeduffyblog.com/2015/11/03/blogging-about-midori/)

```rust
pub struct NetworkCap(());   // ZST — zero runtime cost, cannot be forged
pub struct BlockIoCap(());

// Only kernel can construct — private field prevents external instantiation
pub(crate) fn grant_network_cap() -> NetworkCap { NetworkCap(()) }

// Capability enforced at compile time — no runtime check needed
pub fn tcp_connect(_cap: &NetworkCap, addr: IpAddr) -> ViResult<TcpStream> { ... }
```
