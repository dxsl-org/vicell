//! Cargo build helper — compile `.vi` DSL files to Rust at build time.
//!
//! # Usage in `build.rs`
//!
//! ```rust,ignore
//! fn main() { viui_build::compile("*.vi"); }
//! ```
//!
//! For each `.vi` file found, generates `{stem}.rs` in `$OUT_DIR` and emits a
//! `cargo:rerun-if-changed` directive so incremental builds are correct.
//!
//! # Supported glob patterns (P05)
//!
//! Only single-directory `*.vi` patterns:
//! - `"*.vi"` — all `.vi` files in the crate root
//! - `"src/*.vi"` — all `.vi` files in `src/`
//!
//! Recursive globs (`**/*.vi`) are not supported yet.

use std::path::{Path, PathBuf};
use vi_compiler::codegen::CodeGen;

// ─── Public API ──────────────────────────────────────────────────────────────

/// Compile all `.vi` files matching `glob_pattern` (relative to `CARGO_MANIFEST_DIR`).
///
/// Writes `{component_name}.rs` into `$OUT_DIR`.
/// Emits `cargo:rerun-if-changed=<path>` for each found `.vi` file so incremental
/// builds trigger a recompile when `.vi` source changes.
///
/// # Panics
///
/// Panics — failing the build — if any `.vi` file fails to parse or emit code.
pub fn compile(glob_pattern: &str) {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set — viui_build::compile must be called from build.rs");
    let out_dir = std::env::var("OUT_DIR")
        .expect("OUT_DIR not set — viui_build::compile must be called from build.rs");

    let vi_files = find_vi_files(&manifest_dir, glob_pattern);

    if vi_files.is_empty() {
        println!("cargo:warning=viui-build: no .vi files found for pattern '{glob_pattern}'");
        return;
    }

    let out_path = Path::new(&out_dir);
    for vi_path in &vi_files {
        // Tell Cargo to re-run build.rs when this .vi file changes.
        println!("cargo:rerun-if-changed={}", vi_path.display());
        compile_one(vi_path, out_path);
    }
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// Find `.vi` files matching a simple `{dir_prefix}/*.vi` pattern.
///
/// Returns an empty Vec (without panicking) if the target directory doesn't exist.
fn find_vi_files(manifest_dir: &str, pattern: &str) -> Vec<PathBuf> {
    let base = Path::new(manifest_dir);

    // Split pattern into (directory_part, name_part) at the last '/'
    let (dir, name_glob) = if let Some(pos) = pattern.rfind('/') {
        (&pattern[..pos], &pattern[pos + 1..])
    } else {
        (".", pattern)
    };

    if name_glob != "*.vi" {
        println!(
            "cargo:warning=viui-build: only '*.vi' wildcard supported, got '{name_glob}' — skipping"
        );
        return Vec::new();
    }

    let dir_path = base.join(dir);
    let Ok(entries) = std::fs::read_dir(&dir_path) else {
        return Vec::new();
    };

    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|ext| ext.to_str()) == Some("vi"))
        .collect();

    // Sort for deterministic emit order across platforms.
    files.sort();
    files
}

/// Parse one `.vi` file and write the generated Rust to `out_dir/{stem}.rs`.
fn compile_one(vi_path: &Path, out_dir: &Path) {
    let src = std::fs::read_to_string(vi_path)
        .unwrap_or_else(|e| panic!("viui-build: cannot read {}: {}", vi_path.display(), e));

    let file = vi_compiler::compile_str(&src)
        .unwrap_or_else(|e| panic!("viui-build: parse error in {}: {}", vi_path.display(), e));

    let rust_src = CodeGen::new().generate(&file);

    let stem = vi_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("component");

    let out_file = out_dir.join(format!("{stem}.rs"));
    std::fs::write(&out_file, &rust_src)
        .unwrap_or_else(|e| panic!("viui-build: cannot write {}: {}", out_file.display(), e));
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::find_vi_files;

    #[test]
    fn find_vi_files_missing_dir_returns_empty() {
        let result = find_vi_files("/nonexistent_path_xyz_123_abc", "*.vi");
        assert!(result.is_empty());
    }

    #[test]
    fn find_vi_files_unsupported_wildcard_returns_empty() {
        // Non-*.vi patterns are not supported and must return empty without panic.
        let result = find_vi_files(".", "**/*.vi");
        assert!(result.is_empty());
    }
}
