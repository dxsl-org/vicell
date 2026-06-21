// Build script for the Tetris cell.
//
// Prerequisites (one-time manual steps):
//   Clone Banaxi-Tech/Tetris-OS into src/c/tetris-os/:
//     git clone https://github.com/Banaxi-Tech/Tetris-OS cells/games/tetris-c/src/c/tetris-os
//
// Compiles tetris.c from Tetris-OS plus our vicell_platform.c.
// Platform hooks (vga_*, timer_*, keyboard_*, speaker_*) are implemented
// in src/c/vicell_platform.c — the original hardware drivers are NOT compiled.
//
// The game entry point is tetris_run() declared in tetris.h.  If the clone
// exposes a different symbol, update the extern "C" declaration in src/main.rs
// and the call in vicell_platform.c accordingly.

use std::path::Path;

const TETRIS_OS_DIR: &str = "src/c/tetris-os";

fn main() {
    let dir = Path::new(TETRIS_OS_DIR);
    if !dir.exists() {
        println!(
            "cargo:warning=\
            Tetris-OS source not found at {TETRIS_OS_DIR}. \
            Run: git clone https://github.com/Banaxi-Tech/Tetris-OS \
            cells/games/tetris-c/{TETRIS_OS_DIR}"
        );
        return;
    }

    let target = std::env::var("TARGET").unwrap_or_default();
    let mut build = cc::Build::new();

    // RISC-V bare-metal toolchain
    if target.contains("riscv") {
        if std::env::var("CC_riscv64gc_unknown_none_elf").is_err() {
            build.compiler("riscv-none-elf-gcc");
        }
        build.flag("-mabi=lp64d");
        if std::env::var("AR_riscv64gc_unknown_none_elf").is_err()
            && std::env::var("AR_riscv64gc-unknown-none-elf").is_err()
            && std::env::var("TARGET_AR").is_err()
            && std::env::var("AR").is_err()
        {
            build.archiver("riscv-none-elf-ar");
        }
        let sysroot = run_riscv_gcc(&["--print-sysroot"]);
        if !sysroot.is_empty() && sysroot != "." {
            build.flag(&format!("-I{}/include", sysroot));
        }
    }

    if target.contains("x86_64") || target.contains("aarch64") {
        build.flag("-mcmodel=small");
    }

    build.warnings(false);
    build.flag_if_supported("-std=gnu99");

    // Make tetris-os headers visible for both tetris.c and vicell_platform.c
    build.include(TETRIS_OS_DIR);
    // Make src/c/ visible so vicell_platform.c can #include local helpers if needed
    build.include("src/c");

    // Compile ONLY the pure game logic from tetris-os.
    // Excluded: vga.c, keyboard.c, timer.c, speaker.c (replaced by vicell_platform.c)
    //           main.c (entry provided by Rust main())
    //           Any x86 kernel bootstrap files (idt.c, gdt.c, isr.c, etc.)
    let tetris_c = Path::new(TETRIS_OS_DIR).join("tetris.c");
    if tetris_c.exists() {
        build.file(&tetris_c);
    } else {
        println!(
            "cargo:warning=tetris.c not found in {TETRIS_OS_DIR} — check repo structure."
        );
        return;
    }

    // Our ViCell platform implementation replaces all hardware drivers
    build.file("src/c/vicell_platform.c");

    build.compile("tetris");

    // PIE linker script — kernel assigns VA at spawn time.
    cell_build::emit_linker_script();

    // RISC-V: link picolibc for memset/memcpy builtins used in rendering
    if target.contains("riscv") {
        link_picolibc();
    }

    println!("cargo:rerun-if-changed={TETRIS_OS_DIR}");
    println!("cargo:rerun-if-changed=src/c/vicell_platform.c");
}

fn link_picolibc() {
    let libc   = run_riscv_gcc(&["--print-file-name=libc.a"]);
    let libgcc = run_riscv_gcc(&["--print-libgcc-file-name"]);
    for p in [&libc, &libgcc] {
        if let Some(dir) = std::path::Path::new(p).parent() {
            let d = dir.to_string_lossy();
            if !d.is_empty() && d != "." {
                println!("cargo:rustc-link-search=native={d}");
            }
        }
    }
    println!("cargo:rustc-link-lib=static=c");
    println!("cargo:rustc-link-lib=static=m");
    println!("cargo:rustc-link-lib=static=gcc");
    println!("cargo:rustc-link-arg=--allow-multiple-definition");
}

fn run_riscv_gcc(args: &[&str]) -> String {
    let mut all = vec!["-march=rv64gc", "-mabi=lp64d"];
    all.extend_from_slice(args);
    std::process::Command::new("riscv-none-elf-gcc")
        .args(&all)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}
