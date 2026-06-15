fn main() {
    println!("cargo:rustc-link-arg-bin=input-test=-Tcells/apps/input-test/input-test.ld");
    println!("cargo:rerun-if-changed=cells/apps/input-test/input-test.ld");
}
