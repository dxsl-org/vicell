# ViUI v2 P05 — viui-build: Cargo build.rs Integration

**Plan ID**: 260608-1900-viui-build-p05
**Stage**: G2
**Priority**: P1 — enables automatic `.vi` → Rust code generation in any Cell
**Created**: 2026-06-08
**Depends on**: P04 (vi-compiler codegen pipeline complete)
**Design Brief**: [.agents/brainstorms/260608-viui-nextgen-architecture.md](../brainstorms/260608-viui-nextgen-architecture.md)

---

## Mục tiêu

Cho phép một Cell crate tự động generate Rust từ `.vi` files tại build time:

```toml
# cells/apps/my-app/Cargo.toml
[build-dependencies]
viui-build = { path = "../../../tools/viui-build" }
```

```rust
// cells/apps/my-app/build.rs
fn main() { viui_build::compile("*.vi"); }
```

```rust
// cells/apps/my-app/src/main.rs
include!(concat!(env!("OUT_DIR"), "/counter.rs"));
```

---

## Scope

### In scope
- `tools/viui-build/` — standalone std crate, `viui_build::compile(glob)` API
- `cells/apps/viui-demo/` — demo Cell chứng minh end-to-end pipeline
- `cargo:rerun-if-changed` per `.vi` file — incremental builds
- Single-directory glob pattern (`*.vi`, `src/*.vi`) — no recursive glob

### Out of scope (explicitly deferred)
- **Hot-reload watcher daemon** — cần file watcher + IPC to running app; G2 polish
- **`vi_design!` proc macro** — P06 scope
- **Recursive glob** (`src/**/*.vi`) — add `glob` crate khi thực sự cần (YAGNI)
- **LSP / IDE integration** — separate project
- **Compile error recovery** — P05 fails the build on first error (acceptable)

---

## Architecture

```
tools/viui-build/
├── Cargo.toml     [workspace] standalone — depends on vi-compiler
└── src/
    └── lib.rs     compile(glob) public API

cells/apps/viui-demo/
├── Cargo.toml     [build-dependencies]: viui-build
├── build.rs       viui_build::compile("*.vi")
├── counter.vi     copy from tools/vi-compiler/tests/fixtures/
└── src/
    └── main.rs    include! + Counter::build() + sys_exit
```

### Build flow

```
cargo build -p viui-demo
    ↓
build.rs runs (HOST target, std)
    ↓
viui_build::compile("*.vi")
    ↓ finds counter.vi
    ↓ vi_compiler::compile_str(src) → ViFile AST
    ↓ CodeGen::generate(&file) → Rust source
    ↓ write to $OUT_DIR/counter.rs
    ↓ println!("cargo:rerun-if-changed=counter.vi")
    ↓
Cell main.rs compiled (riscv64 target, no_std)
    ↓
include!("$OUT_DIR/counter.rs") → Counter struct in scope
    ↓
Counter::build() → (Counter { count }, Column)
```

### Why standalone crate (not workspace member)

`viui-build` is `[build-dependencies]` — compiled for HOST target. If it were in the ViCell workspace (target: `riscv64gc-unknown-none-elf`), it would compile correctly as a build-dep, but its own `.cargo/config.toml` would point to the wrong target. Standalone crate pattern (same as `vi-compiler`) avoids this cleanly.

---

## Phase Table

| Phase | File | Nội dung | Status |
|-------|------|----------|--------|
| P01 | [phase-01-viui-build-crate.md](phase-01-viui-build-crate.md) | `tools/viui-build/` crate — `compile()` API + tests | ✅ Complete |
| P02 | [phase-02-demo-cell.md](phase-02-demo-cell.md) | `cells/apps/viui-demo/` + workspace entry + `cargo check` | ✅ Complete |

P02 depends on P01.

---

## Files Created/Modified

```
tools/viui-build/
├── Cargo.toml          (NEW — standalone workspace)
├── .cargo/config.toml  (NEW — override target to host)
└── src/
    └── lib.rs          (NEW)

cells/apps/viui-demo/
├── Cargo.toml          (NEW)
├── build.rs            (NEW)
├── counter.vi          (NEW — copy of test fixture)
└── src/
    └── main.rs         (NEW)

Cargo.toml              (MODIFY — add "cells/apps/viui-demo" to members)
```

---

## Success Criteria

- [x] `cargo check --manifest-path tools/viui-build/Cargo.toml` passes
- [x] `cargo check -p viui-demo` passes (demo cell type-checks)
- [x] `OUT_DIR/counter.rs` contains `pub struct Counter` after build
- [x] `cargo:rerun-if-changed=counter.vi` emitted (incremental build works)
- [x] Modifying `counter.vi` triggers rebuild of `viui-demo`

---

## Evidence

Both phases verified complete:

**P01**: viui-build crate compiles + 2 unit tests pass  
**P02**: viui-demo Cell checks clean + counter.rs generates correctly  
**Integration**: Incremental build detection works — modifying counter.vi triggers rebuild

See individual phase files for detailed evidence.
