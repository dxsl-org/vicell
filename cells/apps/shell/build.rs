use std::env;
use std::path::PathBuf;

fn main() {
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let ld_name = match arch.as_str() {
        "aarch64" => "shell-arm64.ld",
        "x86_64"  => "shell-x86_64.ld",
        _         => "shell.ld",
    };

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let mut path = PathBuf::from(manifest_dir);
    path.pop(); // Go up to apps/
    path.push(ld_name);

    println!("cargo:rustc-link-arg=-T{}", path.display());
    println!("cargo:rerun-if-changed={}", path.display());
}
