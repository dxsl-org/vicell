fn main() {
    println!("cargo:rustc-link-arg=-Tcells/services/input/input.ld");
    println!("cargo:rerun-if-changed=cells/services/input/input.ld");
}
