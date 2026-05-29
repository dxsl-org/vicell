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

    // ViOS glue: portable write/abort/setjmp stubs that do not require newlib.
    build.file(format!("{glue_dir}/lua_vios_glue.c"));

    build.compile("lua54");

    println!("cargo:rerun-if-changed={src_dir}");
    println!("cargo:rerun-if-changed={glue_dir}");
}
