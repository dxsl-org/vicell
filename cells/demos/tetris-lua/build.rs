// Build script for the Tetris-Lua cell.
//
// Compiles Lua 5.4 C sources from cells/runtimes/lua/src/c/src/ (shared sources,
// no duplication of actual game code).  The same ELF-compiler detection and
// picolibc linking logic used by the Lua runtime cell applies here.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(lua_c_unavailable)");

    let target = std::env::var("TARGET").unwrap_or_default();
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();

    if target.contains("x86_64") || target.contains("aarch64") {
        if !has_elf_compiler(&target) {
            println!("cargo:rustc-cfg=lua_c_unavailable");
            println!("cargo:warning=Tetris-Lua: no ELF-capable C compiler found for {target}.");
            println!("cargo:warning=  Install LLVM/Clang and ensure `clang --target={target}-elf` is in PATH.");
            cell_build::emit_linker_script();
            return;
        }
    }

    compile_lua(&target, &manifest);
    cell_build::emit_linker_script();

    if target.contains("riscv") {
        link_picolibc();
    }
}

fn compile_lua(target: &str, manifest: &str) {
    // Lua 5.4 sources live in the lua runtime crate — referenced by absolute path.
    let src_dir  = format!("{manifest}/../../runtimes/lua/src/c/src");
    let glue_dir = format!("{manifest}/../../runtimes/lua/glue");

    // liolib.c / loslib.c are excluded: they pull in FILE* stdio and strftime
    // from picolibc, which contains non-PIC (R_RISCV_64) relocations in
    // _C_time_locale — incompatible with PIE cell linking.  Stubs in
    // src/c/lua_game_stubs.c replace them with empty tables.
    const LUA_SRC: &[&str] = &[
        "lapi.c","lcode.c","lctype.c","ldebug.c","ldo.c","ldump.c",
        "lfunc.c","lgc.c","llex.c","lmem.c","lobject.c","lopcodes.c",
        "lparser.c","lstate.c","lstring.c","ltable.c","ltm.c",
        "lundump.c","lvm.c","lzio.c",
        "lauxlib.c","lbaselib.c","lcorolib.c","ldblib.c",
        "lmathlib.c","loadlib.c","lstrlib.c","ltablib.c",
        "lutf8lib.c","linit.c",
    ];

    let mut build = cc::Build::new();

    if target.contains("riscv") {
        if std::env::var("CC_riscv64gc_unknown_none_elf").is_err() {
            build.compiler("riscv-none-elf-gcc");
        }
        build.flag("-mabi=lp64d");
    }
    if target.contains("x86_64") || target.contains("aarch64") {
        configure_elf_compiler(&mut build, target);
        build.flag("-mcmodel=small");
    }

    build.warnings(false);
    build.flag_if_supported("-std=c99");
    build.include(&src_dir);
    build.include(&glue_dir);
    build.define("LUA_USE_C89", None);

    for file in LUA_SRC {
        build.file(format!("{src_dir}/{file}"));
    }
    build.file(format!("{glue_dir}/lua_vios_glue.c"));
    // Stubs for io/os libs (excluded above to avoid picolibc non-PIC relocations).
    build.file(format!("{manifest}/src/c/lua_game_stubs.c"));
    build.compile("lua54_tl");  // suffix _tl to avoid OUT_DIR collision with lua runtime

    println!("cargo:rerun-if-changed={src_dir}");
    println!("cargo:rerun-if-changed={glue_dir}");
    println!("cargo:rerun-if-changed=scripts/tetris.lua");
    println!("cargo:rerun-if-changed=src/main.rs");
}

fn has_elf_compiler(target: &str) -> bool {
    let env_key = target.replace('-', "_");
    if std::env::var(format!("CC_{env_key}")).is_ok() { return true; }
    let host = std::env::var("HOST").unwrap_or_default();
    if !host.contains("msvc") { return true; }
    let candidates: &[&str] = if target.contains("aarch64") {
        &["clang","clang-18","clang-17","clang-16","aarch64-none-elf-gcc"]
    } else {
        &["clang","clang-18","clang-17","clang-16","x86_64-elf-gcc"]
    };
    candidates.iter().any(|c| std::process::Command::new(c).arg("--version").output().is_ok())
}

fn configure_elf_compiler(build: &mut cc::Build, target: &str) {
    let env_key = target.replace('-', "_");
    if std::env::var(format!("CC_{env_key}")).is_ok() { return; }
    let host = std::env::var("HOST").unwrap_or_default();
    if !host.contains("msvc") { return; }
    for name in &["clang","clang-18","clang-17","clang-16"] {
        if std::process::Command::new(name).arg("--version").output().is_ok() {
            build.compiler(*name);
            let triple = if target.contains("aarch64") {
                "aarch64-unknown-none-elf"
            } else {
                "x86_64-unknown-none-elf"
            };
            build.flag(&format!("--target={triple}"));
            return;
        }
    }
}

fn link_picolibc() {
    fn riscv_gcc(args: &[&str]) -> String {
        let mut a = vec!["-march=rv64gc", "-mabi=lp64d"];
        a.extend_from_slice(args);
        std::process::Command::new("riscv-none-elf-gcc")
            .args(&a).output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    }
    for p in [riscv_gcc(&["--print-file-name=libc.a"]),
              riscv_gcc(&["--print-libgcc-file-name"])] {
        if let Some(dir) = std::path::Path::new(&p).parent() {
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
    println!("cargo:rustc-link-arg=--wrap=_sbrk");
}
