fn main() {
    viui_build::compile("*.vi");
    cell_build::emit_linker_script();
}
