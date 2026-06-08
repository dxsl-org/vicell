# Phase 01 — `tools/viui-build/` Crate

**Plan**: [plan.md](plan.md)
**Status**: ✅ Complete
**Files**:
- `tools/viui-build/Cargo.toml` (NEW)
- `tools/viui-build/.cargo/config.toml` (NEW)
- `tools/viui-build/src/lib.rs` (NEW)

---

## Overview

Build a standalone `std` crate that wraps `vi-compiler` for use as a Cargo `[build-dependency]`.  
Public API: one function `compile(glob_pattern: &str)`.

---

## `Cargo.toml`

```toml
# Standalone crate — NOT part of the ViCell embedded workspace.
# Build tools run on dev machine (host), not riscv64.
[workspace]

[package]
name        = "viui-build"
version     = "0.1.0"
edition     = "2021"
description = "Cargo build helper — compile .vi files to Rust at build time"

[dependencies]
vi-compiler = { path = "../vi-compiler" }
```

---

## `.cargo/config.toml`

```toml
[build]
target = "x86_64-pc-windows-msvc"
```

Same override pattern as `tools/vi-compiler/.cargo/config.toml` — ensures the host target is used, not riscv64 from the parent workspace config.

---

## `src/lib.rs`

### Public API

```rust
/// Compile all `.vi` files matching `glob_pattern` (relative to CARGO_MANIFEST_DIR).
///
/// Writes `{component_name}.rs` files to `OUT_DIR`.
/// Emits `cargo:rerun-if-changed=<path>` for each found `.vi` file.
///
/// # Glob syntax (P05)
/// Simple single-directory patterns only:
/// - `"*.vi"` — all .vi files in crate root
/// - `"src/*.vi"` — all .vi files in src/
///
/// # Panics
/// Panics (fails the build) if any `.vi` file fails to parse or codegen.
pub fn compile(glob_pattern: &str) { ... }
```

### Internal helpers

```rust
/// Find .vi files matching a simple `{dir_prefix}/*.vi` pattern.
fn find_vi_files(manifest_dir: &str, pattern: &str) -> Vec<std::path::PathBuf> { ... }

/// Compile one .vi file → write generated Rust to out_dir.
fn compile_one(vi_path: &std::path::Path, out_dir: &std::path::Path) { ... }
```

### `compile()` implementation logic

```
1. Read env vars: CARGO_MANIFEST_DIR, OUT_DIR (panic if missing)
2. call find_vi_files(manifest_dir, glob_pattern) → Vec<PathBuf>
3. For each vi_path:
   a. println!("cargo:rerun-if-changed={}", vi_path.display())
   b. read_to_string(vi_path)
   c. vi_compiler::compile_str(&src) → on error: panic with filename + error message
   d. vi_compiler::codegen::CodeGen::new().generate(&file)
   e. determine output filename: vi_path.file_stem() + ".rs"
   f. write rust_src to out_dir/output_filename
4. If no .vi files found: emit cargo:warning="viui-build: no .vi files found for {pattern}"
```

### `find_vi_files()` logic

Parse the pattern as `{prefix}/{wildcard}`:
- Split at last `/` to get `dir_part` and `name_part`
- If `name_part == "*.vi"`: scan `manifest_dir/dir_part` with `read_dir`, filter by `.vi` extension
- If no `/`: treat as `*.vi` at crate root

```rust
fn find_vi_files(manifest_dir: &str, pattern: &str) -> Vec<std::path::PathBuf> {
    let base = std::path::Path::new(manifest_dir);
    let (dir, name_glob) = if let Some(pos) = pattern.rfind('/') {
        (&pattern[..pos], &pattern[pos+1..])
    } else {
        (".", pattern)
    };
    
    if name_glob != "*.vi" {
        // Only *.vi wildcard supported in P05
        eprintln!("cargo:warning=viui-build: only *.vi wildcard supported, got '{}'", name_glob);
        return Vec::new();
    }
    
    let dir_path = base.join(dir);
    let Ok(entries) = std::fs::read_dir(&dir_path) else { return Vec::new(); };
    
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("vi"))
        .collect()
}
```

---

## Tests

No unit tests needed in Phase 01 — `vi-compiler` already has 36 tests. Behaviour verified via the demo cell in Phase 02.

Add one integration test to verify `find_vi_files` doesn't panic on missing directory:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn find_vi_files_missing_dir_returns_empty() {
        let result = find_vi_files("/nonexistent_path_xyz", "*.vi");
        assert!(result.is_empty());
    }
}
```

---

## Implementation Steps

1. Create `tools/viui-build/Cargo.toml` (standalone workspace + vi-compiler dep)
2. Create `tools/viui-build/.cargo/config.toml` (host target override)
3. Create `tools/viui-build/src/lib.rs` (compile + helpers + 1 test)
4. Run `cargo check --manifest-path tools/viui-build/Cargo.toml` — must pass
5. Run `cargo test --manifest-path tools/viui-build/Cargo.toml` — 1 test passes

---

## Success Criteria

- [x] `cargo check --manifest-path tools/viui-build/Cargo.toml` passes
- [x] `cargo test --manifest-path tools/viui-build/Cargo.toml` passes (1 test)
- [x] `viui_build::compile` is the only public export
- [x] `cargo clippy -- -D warnings` clean

---

## Evidence

### Build verification

```powershell
cd tools/viui-build
cargo check
# Output: Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.16s ✓

cargo test
# Output: test result: ok. 2 passed; 0 failed ✓
```

### Files created

- `tools/viui-build/Cargo.toml` — standalone workspace with vi-compiler dependency
- `tools/viui-build/.cargo/config.toml` — x86_64-pc-windows-msvc target override
- `tools/viui-build/src/lib.rs` — `compile(glob)` public API + 2 tests

All success criteria met.
