// mlibc-shim build.rs — inject the pre-built mlibc libc.a into the linker.
//
// libc.a must be produced BEFORE a full `cargo build`.  Two paths:
//   Windows:  pwsh scripts/setup-mlibc.ps1   (clones + builds natively)
//   WSL2/Linux: bash scripts/build-mlibc.sh
//
// If libc.a is absent this script emits a Cargo warning and returns early
// so that `cargo check` (no link step) still succeeds.  A full `cargo build`
// of any cell that links mlibc-shim will fail at the linker step with
// "cannot find -lc", which is the intended signal to run the setup script.

use std::env;
use std::path::PathBuf;

fn main() {
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir
        .parent()   // libs/
        .and_then(|p| p.parent())
        .expect("mlibc-shim: could not find workspace root from CARGO_MANIFEST_DIR");

    let build_dir = if arch == "aarch64" {
        workspace_root.join("third_party/mlibc/build-aarch64")
    } else {
        workspace_root.join("third_party/mlibc/build")
    };

    let lib_path = build_dir.join("libc.a");

    if !lib_path.exists() {
        // Non-fatal: warn and skip.  `cargo check` has no link step so this is
        // safe.  `cargo build` will fail at the linker — that error is the cue
        // to run the setup script below.
        println!(
            "cargo:warning=mlibc libc.a missing for arch={arch}. \
             Run `pwsh scripts/setup-mlibc.ps1` (Windows) or \
             `bash scripts/build-mlibc.sh` (WSL2/Linux). \
             Expected: {}",
            lib_path.display()
        );
        return;
    }

    println!("cargo:rustc-link-search=native={}", build_dir.display());
    println!("cargo:rustc-link-lib=static=c");
    println!("cargo:rerun-if-changed={}", lib_path.display());
}
