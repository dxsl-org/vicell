fn main() {
    println!("cargo:rustc-link-arg=-Tcells/services/net/net.ld");
    println!("cargo:rerun-if-changed=cells/services/net/net.ld");
}
