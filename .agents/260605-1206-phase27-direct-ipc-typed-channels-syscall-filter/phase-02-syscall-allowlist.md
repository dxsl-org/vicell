# Phase 02 — Syscall Allowlist

**Status**: 📋 PLANNED  
**Priority**: P1  
**Effort**: 3 days  
**Depends on**: Phase 01 (ViSyscall enum must be stable before bit-index table)

---

## ⚠️ Law 1 Gate — 2x Confirmation Required

Adding `allowlist_bit()` method to `ViSyscall` in `libs/api/src/syscall.rs`. This is a new public method on an existing type — no breaking changes.

---

## Context Links

- Syscall enum: `libs/api/src/syscall.rs` — `ViSyscall` enum + `From<usize>` impl
- Dispatch: `kernel/src/task/syscall.rs:1286` — `ViCell_syscall_dispatch(frame)`
- TCB: `kernel/src/task/tcb.rs` — Task struct
- Loader: `kernel/src/loader.rs` — `spawn_from_path()` where ELF sections are read
- ELF reader: `kernel/src/loader/elf.rs:170-177` — `get_section()` already works for arbitrary named sections
- Raw syscalls 500-503: dispatched in `ViCell_syscall_dispatch`'s `_ => match syscall_id` arm

---

## Overview

Each Cell binary can embed a `u64` bitset declaring which syscalls it needs. The kernel reads this at spawn time from the `__ViCell_syscalls` ELF section and stores it in the TCB. Before dispatching any syscall, the kernel checks the bit — if not set, returns `PermissionDenied` immediately.

**Why u64**: 36 filterable syscalls fit in u64 with 28 spare bits. Stable bit indices (0-35) are independent of raw opcode values (which range 0-411 non-contiguously).

**Default**: `u64::MAX` (permit-all) if the section is absent — backwards compatible, no existing Cell binary breaks.

**Denial**: returns `PermissionDenied` error code; does NOT kill the Cell (matches Tock's philosophy: cooperative recovery is possible).

---

## Critical Design: Allowlist Check BEFORE handle_syscall

`handle_syscall()` internally acquires `SCHEDULER` multiple times. Adding a second `SCHEDULER.lock()` acquisition for the allowlist check BEFORE entering `handle_syscall` creates a double-lock only if the Spinlock is non-reentrant. To avoid this:

**Extract allowlist from TCB at dispatch entry:**

```rust
// In ViCell_syscall_dispatch, BEFORE calling ViCell_syscall_dispatch_inner:
let allowlist = {
    let guard = crate::task::SCHEDULER.lock();
    guard.as_ref()
        .and_then(|s| s.tasks.get(&caller_id))
        .map(|t| t.syscall_allowlist)
        .unwrap_or(u64::MAX)
}; // lock released here

// Now check without holding any lock:
if let Some(bit) = sc.allowlist_bit() {
    if allowlist & (1u64 << bit) == 0 {
        frame.regs[10] = SyscallError::PermissionDenied as usize;
        return;
    }
}
// Then proceed to handle_syscall which acquires its own locks
```

---

## Related Code Files

### Modify
- `libs/api/src/syscall.rs` — add `allowlist_bit() -> Option<u8>` to `ViSyscall`
- `kernel/src/task/tcb.rs` — add `syscall_allowlist: u64` to `Task`
- `kernel/src/loader.rs` — read `__ViCell_syscalls` section in `spawn_from_path()`
- `kernel/src/task/syscall.rs` — add allowlist check at entry of `ViCell_syscall_dispatch`

### Add linker-script KEEP to cell linker scripts
**⚠️ Red-team fix**: `KEEP(*(__ViCell_syscalls))` must go in CELL linker scripts (the binaries that carry the section), NOT in `kernel/linker.ld`. The kernel does not embed cell ELFs directly — adding it there is a no-op. Cell binaries use the default Rust linker (no custom .ld) unless overridden. Add `KEEP` via `build.rs cargo:rustc-link-arg=--keep-section=__ViCell_syscalls` OR ensure cells use a linker script that preserves the section. The safest approach: add `--keep-section=__ViCell_syscalls` to cell build.rs or workspace `.cargo/config.toml` linker flags.

### Provide default allowlists
- `cells/services/vfs/src/main.rs` (or separate file) — declare allowlist static
- `cells/services/net/src/main.rs` — declare allowlist static
- `cells/apps/shell/src/main.rs` — declare allowlist static

---

## Implementation Steps

### Step 1 — Add `allowlist_bit()` to `ViSyscall` (`libs/api/src/syscall.rs`)

```rust
impl ViSyscall {
    /// Stable bit index (0-35) for the per-Cell syscall allowlist stored in
    /// `Task::syscall_allowlist`.  Returns `None` for syscalls that are always
    /// permitted (Yield, Exit) — they are never filtered.
    ///
    /// Bit indices are independent of raw opcode values so they remain stable
    /// even if opcodes are renumbered.
    pub const fn allowlist_bit(self) -> Option<u8> {
        match self {
            Self::Send          => Some(0),
            Self::Recv          => Some(1),
            Self::TryRecv       => Some(2),
            Self::Reply         => Some(3),
            Self::Call          => Some(4),
            Self::Spawn         => Some(5),
            Self::SpawnFromMem  => Some(6),
            Self::SpawnFromPath => Some(7),
            Self::SpawnPinned   => Some(8),
            Self::Wait          => Some(9),
            Self::Log           => Some(10),
            Self::SetTimer      => Some(11),
            Self::ShmAlloc      => Some(12),
            Self::ShmMap        => Some(13),
            Self::GetProcs      => Some(14),
            Self::OpenCap       => Some(15),
            Self::ReadCap       => Some(16),
            Self::CloseCap      => Some(17),
            Self::Open          => Some(18),
            Self::Read          => Some(19),
            Self::Write         => Some(20),
            Self::Close         => Some(21),
            Self::ReadDir       => Some(22),
            Self::Seek          => Some(23),
            Self::FileOp        => Some(24),
            Self::GetTime       => Some(25),
            Self::GpuFlush      => Some(26),
            Self::NetTx         => Some(27),
            Self::NetRx         => Some(28),
            Self::RecvTimeout   => Some(29),
            Self::SendGather    => Some(30),
            Self::RecvScatter   => Some(31),
            Self::HotSwap       => Some(32),
            Self::StateStash    => Some(33),
            Self::StateRestore  => Some(34),
            Self::Exec          => Some(35),
            // Bit 36 is reserved for raw block I/O syscalls (opcodes 500-503)
            // which bypass ViSyscall::from() — see separate raw-id check in dispatch.
            // Yield and Exit are always permitted — filtering them would
            // prevent a Cell from ever yielding or cleanly shutting down.
            Self::Yield | Self::Exit | Self::Unknown => None,
        }
    }
}
```

### Step 2 — Add `syscall_allowlist: u64` to Task (`tcb.rs`)

```rust
// In Task struct, alongside cap fields:
/// Per-Cell syscall allowlist.  Bit N = syscall with `allowlist_bit() == N` is
/// permitted.  `u64::MAX` = permit-all (default when ELF section is absent).
pub syscall_allowlist: u64,
```

In `Task::new()`: `syscall_allowlist: u64::MAX`.

### Step 3 — Read ELF section in `loader.rs`

```rust
// In spawn_from_path(), after tid is assigned:
let allowlist = {
    let loader = crate::loader::elf::ElfLoader;
    // get_section already uses xmas-elf find_section_by_name — no new dep needed
    match loader.get_section(&elf_bytes, "__ViCell_syscalls") {
        Ok(bytes) if bytes.len() >= 8 => {
            // SAFETY: bytes is a valid u8 slice from ELF section data.
            u64::from_le_bytes(bytes[..8].try_into().expect("8-byte slice"))
        }
        _ => u64::MAX, // section absent → permit-all (backwards compatible)
    }
};
if let Some(sched) = crate::task::SCHEDULER.lock().as_mut() {
    if let Some(task) = sched.tasks.get_mut(&tid) {
        task.syscall_allowlist = allowlist;
    }
}
```

### Step 4 — Check at dispatch entry (`syscall.rs`)

At the very start of `ViCell_syscall_dispatch`, before the `let syscall = ...` decode:

```rust
// Extract allowlist then drop lock — avoids holding SCHEDULER while handle_syscall
// acquires it internally.
let caller_allowlist = {
    let guard = super::SCHEDULER.lock();
    guard.as_ref()
        .and_then(|s| s.tasks.get(&caller_id))
        .map(|t| t.syscall_allowlist)
        .unwrap_or(u64::MAX)
}; // lock released

let syscall_id = frame.regs[17]; // a7
let sc = api::syscall::ViSyscall::from(syscall_id);
if let Some(bit) = sc.allowlist_bit() {
    if caller_allowlist & (1u64 << bit) == 0 {
        frame.regs[10] = SyscallError::PermissionDenied as usize;
        return;
    }
}
// Handle raw 500-503 (BlkRead/Write/BlkFlush) — they bypass ViSyscall::from()
// but still participate in the allowlist via bit 36 (user confirmed in Phase 27).
if matches!(syscall_id, 500 | 501 | 503) {
    if caller_allowlist & (1u64 << 36) == 0 {
        frame.regs[10] = SyscallError::PermissionDenied as usize;
        return;
    }
    // BlockIoCap check still applies (Phase 26) — both must pass.
}
```

### Step 5 — Cell-side allowlist declarations

Each Cell declares its allowlist via a link-section static:

```rust
// In cells/services/vfs/src/main.rs — VFS needs Recv, Reply, Log, Open, Read, Write etc.
#[used]
#[link_section = "__ViCell_syscalls"]
static VFS_SYSCALL_ALLOWLIST: u64 =
    (1 << 1)   // Recv
    | (1 << 3) // Reply
    | (1 << 10)// Log
    | (1 << 18)// Open
    | (1 << 19)// Read
    | (1 << 20)// Write
    | (1 << 21)// Close
    | (1 << 22)// ReadDir
    | (1 << 23)// Seek
    | (1 << 24)// FileOp
    | (1 << 60);// Exit (bit 60 ≠ any defined bit → always permitted anyway)
```

Add `KEEP(*(__ViCell_syscalls))` to `kernel/linker.ld` and each cell linker script.

---

## Todo List

- [ ] ⚠️ Confirm `allowlist_bit()` method on ViSyscall (Law 1, 2x required)
- [ ] Add `allowlist_bit()` to `ViSyscall` in `libs/api/src/syscall.rs`
- [ ] Add `syscall_allowlist: u64` to Task, default `u64::MAX`
- [ ] Read `__ViCell_syscalls` ELF section in `loader.rs`
- [ ] Add allowlist check at start of `ViCell_syscall_dispatch` (lock-drop pattern)
- [ ] Add `KEEP(*(__ViCell_syscalls))` to `kernel/linker.ld`
- [ ] Declare VFS, net, shell allowlists in respective cell main.rs files
- [ ] `cargo check --workspace` — no errors
- [ ] Test: cell without Recv bit calls sys_recv → PermissionDenied returned

---

## Success Criteria

- [ ] `ViSyscall::Recv.allowlist_bit()` returns `Some(1)`
- [ ] Task struct compiles with `syscall_allowlist: u64` field
- [ ] VFS spawned via `spawn_from_path` has its allowlist from ELF section
- [ ] A test cell with empty allowlist (0x0) returns PermissionDenied on any non-exempt syscall
- [ ] All 65 integration tests pass (existing cells have u64::MAX default)

---

## Risk Assessment

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| `KEEP(*(__ViCell_syscalls))` missing from linker scripts → section silently dropped | Medium | Add to kernel/linker.ld AND verify with `objdump -h cell_binary \| grep ViCell` |
| Raw 500-503 opcodes bypass `ViSyscall::from()` → allowlist check skipped | Confirmed | Handle in separate raw-id check before the `ViSyscall::from()` match |
| SCHEDULER double-lock if allowlist check is inside handle_syscall | Confirmed | Lock-drop pattern: read allowlist + drop lock before calling handle_syscall |
| Permit-all default masks real security gain during transition | Low | Acceptable: each service gradually declares minimal allowlist; full enforcement later |
