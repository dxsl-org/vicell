// Build script for the DOOM cell.
//
// Prerequisites (one-time manual steps):
//   1. Clone doomgeneric into src/c/doomgeneric/:
//        git clone https://github.com/ozkl/doomgeneric cells/apps/doom/src/c/doomgeneric
//   2. Copy doom1.wad (shareware) to the ViCell FAT32 disk image as /doom1.wad.
//
// The build compiles all doomgeneric .c files, renaming main() to __doom_c_entry
// so the Rust ostd entry point owns the process start.  Platform hooks (DG_Init,
// DG_DrawFrame, DG_GetKey, DG_GetTicksMs, DG_SleepMs, DG_Quit) are implemented
// as #[no_mangle] extern "C" functions in src/main.rs — no doomgeneric_*.c needed.

use std::path::Path;
use std::fs;

// The ozkl/doomgeneric repo nests the C sources one level deeper:
// cells/apps/doom/src/c/doomgeneric/  ← git repo root (has README, .sln)
//                                doomgeneric/  ← actual C source
const DOOMGENERIC_DIR: &str = "src/c/doomgeneric/doomgeneric";

fn main() {
    let dg_path = Path::new(DOOMGENERIC_DIR);
    if !dg_path.exists() {
        // Emit a Cargo warning (not an error) so `cargo check` can still
        // type-check the Rust source.  A full `cargo build` will fail at the
        // link step with unresolved C symbols — that's intentional.
        println!(
            "cargo:warning=\
            doomgeneric source not found at {DOOMGENERIC_DIR}. \
            Run: git clone https://github.com/ozkl/doomgeneric {DOOMGENERIC_DIR}"
        );
        // Emit a dummy empty archive so the linker step does not fail on
        // cargo check (no linking happens there anyway — this is future-proofing).
        return;
    }

    let target = std::env::var("TARGET").unwrap_or_default();

    // Collect all .c files from doomgeneric/, excluding:
    //  - doomgeneric_*.c  : example platform impls (SDL/Xlib/etc.) — we provide DG_* in Rust
    //  - i_allegromusic.c / i_allegrosound.c : Allegro sound backends (require allegro headers)
    //  - i_sdlmusic.c / i_sdlsound.c         : SDL sound backends (require SDL headers)
    // Keep doomgeneric.c itself (owns DG_ScreenBuffer + doomgeneric_Create).
    const EXCLUDED: &[&str] = &[
        "i_allegromusic", "i_allegrosound",
        "i_sdlmusic",     "i_sdlsound",
    ];
    let c_files: Vec<_> = fs::read_dir(dg_path)
        .expect("read doomgeneric dir")
        .filter_map(|e| {
            let e = e.ok()?;
            let name = e.file_name().into_string().ok()?;
            if !name.ends_with(".c") { return None; }
            let stem = name.trim_end_matches(".c");
            if stem.starts_with("doomgeneric_") { return None; }
            if EXCLUDED.contains(&stem) { return None; }
            Some(e.path())
        })
        .collect();

    let mut build = cc::Build::new();

    // RISC-V bare-metal toolchain
    if target.contains("riscv") {
        if std::env::var("CC_riscv64gc_unknown_none_elf").is_err() {
            build.compiler("riscv-none-elf-gcc");
        }
        build.flag("-mabi=lp64d");
        // cc-rs auto-detects `ar` from PATH but on Windows it may find the
        // MinGW archiver which cannot process RISC-V ELF objects. Force the
        // cross-archiver explicitly, matching the compiler.
        if std::env::var("AR_riscv64gc_unknown_none_elf").is_err()
            && std::env::var("AR_riscv64gc-unknown-none-elf").is_err()
            && std::env::var("TARGET_AR").is_err()
            && std::env::var("AR").is_err()
        {
            build.archiver("riscv-none-elf-ar");
        }
        // DOOM uses strdup, snprintf, etc. — point the compiler at picolibc
        // headers so the full POSIX subset is visible (not just freestanding).
        let sysroot = run_gcc(&["--print-sysroot"]);
        if !sysroot.is_empty() && sysroot != "." {
            build.flag(&format!("-I{}/include", sysroot));
        }
    }

    // Override code-model for cells (well below 2GB)
    if target.contains("x86_64") || target.contains("aarch64") {
        build.flag("-mcmodel=small");
    }

    build.warnings(false);
    // gnu99 instead of c99 so __STRICT_ANSI__ is off → picolibc exposes
    // strdup, snprintf, and other POSIX-visible functions in its headers.
    build.flag_if_supported("-std=gnu99");

    // doomgeneric exposes doomgeneric_Create + doomgeneric_Tick; we call
    // them from Rust main() directly, so no main()-rename trick needed.

    // Standard DOOM defines for doomgeneric
    build.define("DOOMGENERIC_RESX", Some("320"));
    build.define("DOOMGENERIC_RESY", Some("200"));
    // No sound by default (no audio backend yet)
    build.define("NOSOUND", None);

    build.include(DOOMGENERIC_DIR);

    for path in &c_files {
        build.file(path);
    }

    build.compile("doom");

    // Arch-specific linker script
    let ld = if target.contains("aarch64") {
        "cells/apps/doom/doom-arm64.ld"
    } else if target.contains("x86_64") {
        "cells/apps/doom/doom-x86_64.ld"
    } else {
        "cells/apps/doom/doom.ld"
    };
    println!("cargo:rustc-link-arg=-T{ld}");
    println!("cargo:rerun-if-changed={ld}");

    // RISC-V needs picolibc for sprintf/strtod/etc. used heavily in DOOM
    if target.contains("riscv") {
        link_picolibc();
    }

    println!("cargo:rerun-if-changed={DOOMGENERIC_DIR}");
}

fn link_picolibc() {
    let libc   = run_gcc(&["--print-file-name=libc.a"]);
    let libgcc = run_gcc(&["--print-libgcc-file-name"]);
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
    // No --wrap=_sbrk: _sbrk is already provided by libs/api/src/posix/sysio.rs
    // (returns NULL — Rust GlobalAlloc owns the heap in SAS).
}

fn run_gcc(args: &[&str]) -> String {
    let mut all = vec!["-march=rv64gc", "-mabi=lp64d"];
    all.extend_from_slice(args);
    std::process::Command::new("riscv-none-elf-gcc")
        .args(&all)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}
