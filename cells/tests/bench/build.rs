fn main() {
    // Both bench and bench-probe use the same PIE linker script.
    // The kernel assigns distinct VA bases at spawn time, so both binaries
    // can coexist in the SAS page table without VA collision.
    cell_build::emit_linker_script();
}
