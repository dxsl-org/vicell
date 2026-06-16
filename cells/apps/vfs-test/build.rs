fn main() {
    println!("cargo:rustc-link-arg=-Tcells/apps/vfs-test/vfs-test.ld");
    println!("cargo:rerun-if-changed=cells/apps/vfs-test/vfs-test.ld");
}
