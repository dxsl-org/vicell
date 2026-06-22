# Tier 1b C/C++/Zig — C ABI via POSIX or mlibc

> Call C libraries from Rust Cells, or link C code directly. Two strategies: Tier A (POSIX shim) or Tier B (full mlibc).

---

## Tier A vs Tier B

| | **Tier A: POSIX Shim** | **Tier B: Full mlibc** |
|---|---|---|
| **Setup** | `api = { features = ["posix"] }` | Requires `scripts/build-mlibc.sh` in WSL2; then `api = { features = ["mlibc"] }` |
| **Function coverage** | ~20 POSIX symbols (getentropy, socket, printf, malloc) | Full POSIX + glibc extensions |
| **Link size** | ~5 KB | ~400 KB |
| **Use case** | Quick C interop for simple functions | Heavy C code (curl, zlib, sqlite, etc.) |
| **Complexity** | Low | High (mlibc build in separate shell) |

**Critical**: never enable both features. They are **mutually exclusive**.

---

## Tier A: POSIX Shim

Minimal C ABI for common functions. Declared in `libs/api/src/posix.rs`.

### Setup

```rust
// Cargo.toml
[dependencies]
api = { path = "libs/api", features = ["posix"] }

// main.rs
extern "C" {
    fn malloc(size: usize) -> *mut u8;
    fn free(ptr: *mut u8);
    fn printf(fmt: *const u8, ...) -> i32;
    fn socket(domain: i32, socktype: i32, protocol: i32) -> i32;
    fn getentropy(buf: *mut u8, len: usize) -> i32;
    fn clock_gettime(clock_id: i32, tp: *mut TimeSpec) -> i32;
}
```

### Available Functions

- `malloc(size)` → heap allocation via `sys_anon_allocate`
- `free(ptr)` → deallocate
- `printf(fmt, ...)` → formatted output
- `socket(domain, socktype, protocol)` → TCP/UDP socket
- `connect(fd, addr, addrlen)` → connect to peer
- `send(fd, buf, len, flags)` → send bytes
- `recv(fd, buf, len, flags)` → receive bytes
- `close(fd)` → close socket
- `getentropy(buf, len)` → random bytes (via `sys_get_random`)
- `clock_gettime(clock_id, tp)` → wall-clock / monotonic time

For the full list, see `libs/api/src/posix.rs`.

### Example: Getentropy

```rust
use core::ffi::c_void;

extern "C" {
    fn getentropy(buf: *mut c_void, len: usize) -> i32;
}

unsafe {
    let mut random = [0u8; 32];
    if getentropy(random.as_mut_ptr() as *mut c_void, 32) == 0 {
        // random[] filled with 32 bytes of entropy
    }
}
```

---

## Tier B: Full mlibc

Complete C standard library via mlibc (libc.a). Supports fork(), pthread, complex math, etc.

### Build mlibc (One-Time Setup)

On **Windows in WSL2**:

```bash
# In Cellos root
cd scripts
bash build-mlibc.sh

# Check result: should create mlibc/aarch64-Cellos/lib/libc.a (and other targets)
ls mlibc/aarch64-Cellos/lib/libc.a
```

Mlibc is **git-ignored**; it's rebuilt as part of the kernel build. No commit needed.

### Setup

```rust
// Cargo.toml
[dependencies]
api = { path = "libs/api", features = ["mlibc"] }

// Manifest: block_io false (unless you need raw disk)
api::declare_manifest!(block_io = false, network = false, spawn = false);

// main.rs
extern "C" {
    fn printf(fmt: *const u8, ...) -> i32;
    fn malloc(size: usize) -> *mut u8;
    fn free(ptr: *mut u8);
    fn clock_gettime(clock_id: i32, tp: *mut libc::timespec) -> i32;
    fn sqrt(x: f64) -> f64;
    // ... any C symbol
}
```

### Example: Complex Math

```rust
extern "C" {
    fn sqrt(x: f64) -> f64;
    fn sin(x: f64) -> f64;
}

fn main() {
    unsafe {
        let result = sqrt(16.0);  // 4.0
        let sine = sin(3.14159 / 2.0);  // ~1.0
    }
}
```

### Common Functions

- **Stdio**: `printf`, `fprintf`, `sprintf`, `vprintf` (buffering via syscalls)
- **Memory**: `malloc`, `calloc`, `realloc`, `free`
- **String**: `strlen`, `strcpy`, `strcmp`, `strtok`, `snprintf`
- **Math**: `sqrt`, `sin`, `cos`, `exp`, `log`
- **Time**: `clock_gettime`, `gettimeofday`
- **Entropy**: `getentropy`
- **Network**: `socket`, `connect`, `send`, `recv`, `close` (as in Tier A)

---

## Mutual Exclusion (CRITICAL)

**Never do this:**

```rust
#[cfg(feature = "posix")]
extern "C" { fn my_func(); }

#[cfg(feature = "mlibc")]
extern "C" { fn my_func(); }
```

If both features are enabled, the linker will fail with duplicate symbols or undefined references. **Pick one and stick with it.**

Use a build.rs to enforce exclusivity:

```rust
// build.rs
fn main() {
    let posix = cfg!(feature = "posix");
    let mlibc = cfg!(feature = "mlibc");
    if posix && mlibc {
        panic!("cannot enable both 'posix' and 'mlibc' features");
    }
}
```

---

## C Runtime Constraints

Cellos SAS laws apply to C code too (since it's in a Rust Cell):

❌ **Fork / subprocess spawning** — SAS has no fork. Use `spawn = true` manifest + `sys_spawn` (Tier 1 Rust only).  
❌ **Mmap** — No virtual memory per-cell. Use heap (malloc) or VFS.  
❌ **Signals / SIGCHLD** — Not applicable in SAS.  
✅ **Pthreads** — Supported via `sys_task_spawn` (POSIX threads map to kernel tasks).  
✅ **Sockets** — Full support via POSIX shim or mlibc.  

---

## Manifest & Syscalls

```rust
api::declare_manifest!(
    block_io = false,     // Use VFS, not raw disk
    network = true,       // If using sockets
    spawn = false         // Only if you're init/shell
);

api::declare_syscalls![
    Send, Recv, Log, Exit,
    GetTime,
    GetRandom,
    LookupService,
    AnonAllocate  // for malloc
];
```

---

## Canonical Examples

- **Tier A (POSIX shim)**: [cells/apps/posix-shim-test/src/main.rs](../../cells/apps/posix-shim-test/src/main.rs) — getentropy, socket, connect, send/recv.
- **Tier B (mlibc)**: [cells/apps/mlibc-smoke/src/main.rs](../../cells/apps/mlibc-smoke/src/main.rs) — malloc, printf, clock_gettime.

---

## When to Use Tier 1b

✅ Have existing C/C++/Zig code  
✅ Need glibc functions (complex math, pthreads, stdio)  
✅ Interfacing with a library (zlib, curl, etc.)  

❌ Building from scratch in Rust → stay with Tier 1  
❌ Need untrusted code isolation → use Tier 3b (Linux VM)  

---

## Build & Run

```bash
# Tier A (POSIX shim) — no special build
cargo build --release --target riscv64gc-unknown-none-elf

# Tier B (mlibc) — requires WSL2 + build-mlibc.sh
# (already built by kernel/Makefile, linked automatically)
cargo build --release --target riscv64gc-unknown-none-elf
```

---

## Troubleshooting

**Linker error: undefined reference to `sqrt`?**  
→ You're using Tier A. For math functions, build mlibc (Tier B) or implement them in Rust.

**Both features enabled?**  
→ The linker fails with duplicate symbol errors. Remove one feature from Cargo.toml.

**Malloc returns null?**  
→ Heap exhausted (cell quota too small). See [code-standards.md](../code-standards.md) § Cell quotas.

---

## Next Steps

- Need to write unsafe code? → Keep it in C via Tier 1b.
- Need UIs in Rust? → [Tier 1 + ViUI](viui-guide.md)
- Need cryptographic keys? → [Tier 1 Extended (Silo)](tier1-silo.md)
- See [mlibc-build.md](../mlibc-build.md) for mlibc compilation details.
