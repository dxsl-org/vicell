fn main() {
    // Per-binary linker scripts: each binary gets a distinct VA base so that
    // the orchestrator (bench) and its probe/load children (bench-probe) can
    // coexist in the SAS page table without VA collision.
    println!("cargo:rustc-link-arg-bin=bench=-Tcells/apps/bench/bench.ld");
    println!("cargo:rustc-link-arg-bin=bench-probe=-Tcells/apps/bench/bench-probe.ld");
    println!("cargo:rerun-if-changed=cells/apps/bench/bench.ld");
    println!("cargo:rerun-if-changed=cells/apps/bench/bench-probe.ld");
}
