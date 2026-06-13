fn main() {
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let ld = if arch == "x86_64" {
        "cells/services/vfs/vfs-x86_64.ld"
    } else {
        "cells/services/vfs/vfs.ld"
    };
    println!("cargo:rustc-link-arg=-T{ld}");
    // littlefs2-sys vendors a freestanding string.c (strlen/strchr/strspn/strcspn)
    // that collides with the identical symbols in api's POSIX shim (posix.rs) —
    // the shim must keep them for Tier-1b cells that don't link littlefs.
    // muldefs keeps the first definition (the Rust shim's), which is equivalent.
    println!("cargo:rustc-link-arg=-zmuldefs");
    println!("cargo:rerun-if-changed={ld}");
    println!("cargo:rerun-if-changed=cells/services/vfs/vfs.ld");
}
