fn main() {
    println!("cargo:rustc-link-arg=-Tcells/apps/sdk-demo/sdk-demo.ld");
    println!("cargo:rerun-if-changed=cells/apps/sdk-demo/sdk-demo.ld");
}
