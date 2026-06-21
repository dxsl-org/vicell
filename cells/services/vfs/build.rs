fn main() {
    cell_build::emit_linker_script();
    // littlefs2-sys vendors a freestanding string.c (strlen/strchr/strspn/strcspn)
    // that collides with the identical symbols in api's POSIX shim (posix.rs) —
    // the shim must keep them for Tier-1b cells that don't link littlefs.
    // muldefs keeps the first definition (the Rust shim's), which is equivalent.
    println!("cargo:rustc-link-arg=-zmuldefs");
}
