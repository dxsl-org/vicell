fn main() {
    // List all Lua 5.4.x C source files (excludes lua.c / luac.c which define main).
    let lua_src = [
        "lapi.c", "lcode.c", "lctype.c", "ldebug.c", "ldo.c", "ldump.c",
        "lfunc.c", "lgc.c", "llex.c", "lmem.c", "lobject.c", "lopcodes.c",
        "lparser.c", "lstate.c", "lstring.c", "ltable.c", "ltm.c",
        "lundump.c", "lvm.c", "lzio.c",
        "lauxlib.c", "lbaselib.c", "lcorolib.c", "ldblib.c", "liolib.c",
        "lmathlib.c", "loadlib.c", "loslib.c", "lstrlib.c", "ltablib.c",
        "lutf8lib.c", "linit.c",
    ];

    let src_dir = "src/c/src";
    let glue_dir = "glue";

    let mut build = cc::Build::new();

    // On RISC-V bare-metal the canonical ViOS toolchain is riscv-none-elf-gcc
    // (xpack release).  The cc crate's auto-detection looks for
    // riscv64-unknown-elf-gcc which is not available; set it explicitly unless
    // the caller has already set CC_<target> in the environment.
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("riscv") && std::env::var("CC_riscv64gc_unknown_none_elf").is_err() {
        build.compiler("riscv-none-elf-gcc");
        // Required ABI flag for LP64D (64-bit ints/ptrs, double-precision FP).
        build.flag("-mabi=lp64d");
    }

    // Silence warnings from upstream Lua (we don't own these sources).
    build.warnings(false);

    // Compile as C99 — Lua 5.4 uses C99 features.
    build.flag_if_supported("-std=c99");

    // Use the portable glue config instead of Lua's default luaconf.h macros.
    build.include(src_dir);
    build.include(glue_dir);
    build.define("LUA_USE_C89", None); // Disables POSIX-only features

    for file in &lua_src {
        build.file(format!("{src_dir}/{file}"));
    }

    // ViOS glue: vios_write/abort + safe POSIX stubs (system/getenv/tmpnam).
    build.file(format!("{glue_dir}/lua_vios_glue.c"));

    build.compile("lua54");

    // Place the cell at a unique user-space VA (0x0C000000). Without this the
    // linker defaults to 0x10000, which overlapped other mappings and stopped
    // the spawned cell from running.
    println!("cargo:rustc-link-arg=-Tcells/runtimes/lua/lua.ld");
    println!("cargo:rerun-if-changed=cells/runtimes/lua/lua.ld");

    // Link picolibc/newlib from the riscv-none-elf-gcc toolchain so the C
    // runtime symbols Lua needs (setjmp, longjmp, strchr, frexp, fwrite,
    // strtod, sprintf, _impure_ptr, ...) resolve.  Without this the cell
    // fails to link with "undefined symbol: setjmp" etc.
    if target.contains("riscv") {
        link_picolibc();
    }

    println!("cargo:rerun-if-changed={src_dir}");
    println!("cargo:rerun-if-changed={glue_dir}");
}

/// Locate picolibc/newlib + libgcc from riscv-none-elf-gcc and emit the
/// linker flags needed to resolve C runtime symbols.  Uses
/// `--allow-multiple-definition` because compiler_builtins (from Rust core)
/// also defines memcpy/memset/etc.; compiler_builtins wins (linked first).
fn link_picolibc() {
    let libc = run_gcc(&["--print-file-name=libc.a"]);
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
    // Redirect every `_sbrk` reference (including libc's internal `_sbrk_r`)
    // to our heap-backed `__wrap__sbrk`. The toolchain's `_sbrk` stub returns
    // null, which faults malloc; --wrap wins regardless of link order, unlike
    // a plain symbol override under --allow-multiple-definition.
    println!("cargo:rustc-link-arg=--wrap=_sbrk");
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
