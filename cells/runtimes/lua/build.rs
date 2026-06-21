fn main() {
    // Declare the cfg so rustc's check-cfg lint doesn't warn about it.
    println!("cargo::rustc-check-cfg=cfg(lua_c_unavailable)");

    let target = std::env::var("TARGET").unwrap_or_default();

    // For x86_64 and aarch64 we need an ELF-capable C compiler.
    // On Windows + MSVC the cc crate defaults to cl.exe (COFF output) which is
    // incompatible with rust-lld for bare-metal ELF targets. Detect and handle
    // this case gracefully before attempting compilation.
    if target.contains("x86_64") || target.contains("aarch64") {
        if !has_elf_compiler(&target) {
            // Emit a cfg flag so Rust code can compile a stub instead of the real
            // Lua runtime. The cell will link and run but print a "not available"
            // message. This keeps workspace builds green on Windows dev machines.
            println!("cargo:rustc-cfg=lua_c_unavailable");
            println!("cargo:warning=Lua cell: no ELF-capable C compiler found for {target}.");
            println!("cargo:warning=  Install LLVM/Clang (https://releases.llvm.org) and ensure");
            println!("cargo:warning=  `clang --target={target}-elf` is in PATH, or set:");
            if target.contains("aarch64") {
                println!("cargo:warning=  CC_aarch64_unknown_none=aarch64-none-elf-gcc");
            } else {
                println!("cargo:warning=  CC_x86_64_unknown_none=x86_64-elf-gcc");
            }
            println!("cargo:warning=  Lua cell will build as a no-op stub for this target.");
            cell_build::emit_linker_script();
            return;
        }
    }

    compile_lua_c(&target);
    cell_build::emit_linker_script();

    // riscv64: link picolibc for C runtime symbols not yet covered by posix.rs
    // (strtod, _impure_ptr, libc internals). arm64 + x86_64 use posix.rs directly.
    if target.contains("riscv") {
        link_picolibc();
    }
}

fn compile_lua_c(target: &str) {
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
    // (xpack release). The cc crate's auto-detection looks for
    // riscv64-unknown-elf-gcc which is not available; set it explicitly unless
    // the caller has already set CC_<target> in the environment.
    if target.contains("riscv") && std::env::var("CC_riscv64gc_unknown_none_elf").is_err() {
        build.compiler("riscv-none-elf-gcc");
        // Required ABI flag for LP64D (64-bit ints/ptrs, double-precision FP).
        build.flag("-mabi=lp64d");
    }

    // For x86_64/aarch64 we already checked has_elf_compiler() above, so here
    // we know either a cross-gcc is in PATH or CC_<target> is set. If using
    // clang, configure it to emit ELF for the right target.
    if target.contains("x86_64") || target.contains("aarch64") {
        configure_elf_compiler(&mut build, target);
        // .cargo/config.toml sets code-model=kernel for x86_64-unknown-none; cells
        // live at 0x1C000000 (well below 2 GB) so "small" is correct.
        build.flag("-mcmodel=small");
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

    println!("cargo:rerun-if-changed={src_dir}");
    println!("cargo:rerun-if-changed={glue_dir}");
}

/// Returns true if an ELF-capable C compiler is available for `target`.
///
/// On Linux/macOS the cc crate finds GCC/clang naturally and they always
/// produce ELF. On Windows + MSVC the cc crate picks cl.exe (COFF), so we
/// must explicitly find a cross-compiler.
fn has_elf_compiler(target: &str) -> bool {
    // If the caller set CC_<target> explicitly, trust them.
    let env_key = target.replace('-', "_");
    if std::env::var(format!("CC_{env_key}")).is_ok() {
        return true;
    }

    // Not on MSVC? The default compiler will produce ELF.
    let host = std::env::var("HOST").unwrap_or_default();
    if !host.contains("msvc") {
        return true;
    }

    // On MSVC: look for clang (any version) or arch-specific cross-gcc.
    let candidates: &[&str] = if target.contains("aarch64") {
        &["clang", "clang-18", "clang-17", "clang-16", "aarch64-none-elf-gcc", "aarch64-linux-gnu-gcc"]
    } else {
        &["clang", "clang-18", "clang-17", "clang-16", "x86_64-elf-gcc", "x86_64-linux-gnu-gcc"]
    };

    candidates.iter().any(|c| {
        std::process::Command::new(c)
            .arg("--version")
            .output()
            .is_ok()
    })
}

/// Configure `build` to use the best available ELF cross-compiler for `target`.
/// Only called after `has_elf_compiler()` returned true.
fn configure_elf_compiler(build: &mut cc::Build, target: &str) {
    // If an explicit CC env var is set, cc crate picks it up automatically.
    let env_key = target.replace('-', "_");
    if std::env::var(format!("CC_{env_key}")).is_ok() {
        return;
    }

    // On MSVC we need to point cc at a real ELF compiler. Try clang first
    // (it can target any ELF triple natively).
    let host = std::env::var("HOST").unwrap_or_default();
    if host.contains("msvc") {
        let clang_names = ["clang", "clang-18", "clang-17", "clang-16"];
        for name in &clang_names {
            if std::process::Command::new(name).arg("--version").output().is_ok() {
                build.compiler(*name);
                // Tell clang to emit ELF for the bare-metal target.
                let elf_triple = if target.contains("aarch64") {
                    "aarch64-unknown-none-elf"
                } else {
                    "x86_64-unknown-none-elf"
                };
                build.flag(&format!("--target={elf_triple}"));
                return;
            }
        }
        // Fallback: cross-gcc (aarch64-none-elf-gcc / x86_64-elf-gcc).
        // These already target ELF natively — no --target flag needed.
    }
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
