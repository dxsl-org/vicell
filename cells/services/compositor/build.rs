fn main() {
    println!("cargo:rustc-link-arg=-Tcells/services/compositor/compositor.ld");
    println!("cargo:rerun-if-changed=cells/services/compositor/compositor.ld");
}
