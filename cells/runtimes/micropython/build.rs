use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let vendor = manifest.join("vendor");
    let port_dir = manifest.join("src/c/vios");
    let genhdr_dir = manifest.join("src/c/genhdr");
    let embed_port = vendor.join("ports/embed/port");

    // ── Step 1: generate genhdr/ if missing or stale ─────────────────────
    // Pass the port C dir as arg 3 so gen_genhdr.py scans modvnet.c for
    // MP_QSTR_ references and MP_REGISTER_MODULE registrations.
    let qstr_out = genhdr_dir.join("qstrdefs.generated.h");
    if !qstr_out.exists() {
        let python = find_python();
        let gen_script = manifest.join("gen_genhdr.py");
        let status = Command::new(&python)
            .arg(&gen_script)
            .arg(&vendor)
            .arg(&genhdr_dir)
            .arg(&port_dir)
            .status()
            .expect("gen_genhdr.py: failed to run");
        assert!(status.success(), "gen_genhdr.py exited with error");
    }

    // ── Step 2: configure cc build ────────────────────────────────────────
    let mut build = cc::Build::new();

    // Cross-compiler detection (same as Lua crate).
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("riscv") && std::env::var("CC_riscv64gc_unknown_none_elf").is_err() {
        build.compiler("riscv-none-elf-gcc");
        build.flag("-mabi=lp64d");
    }

    build.warnings(false);
    build.flag_if_supported("-std=c99");
    build.flag_if_supported("-Wno-implicit-function-declaration");
    build.flag_if_supported("-fno-builtin-printf");

    // Include root of vendor/ so `#include "py/..."` resolves correctly.
    build.include(&vendor);
    // Port config (mpconfigport.h lives here).
    build.include(&port_dir);
    // Generated headers (genhdr/qstrdefs.generated.h etc.).
    build.include(&manifest.join("src/c"));
    // Embed port directory: embed_util.c does `#include "port/micropython_embed.h"`
    // so we need both the parent dir and the port/ dir on the include path.
    build.include(vendor.join("ports/embed"));  // resolves "port/micropython_embed.h"
    build.include(&embed_port);                 // resolves "mphalport.h" etc.

    // ── Step 3: py/ core sources (bytecode-only, RV64 NLR via setjmp) ────
    let py = vendor.join("py");
    let py_sources: &[&str] = &[
        // Core state + memory
        "mpstate.c", "malloc.c", "gc.c", "pystack.c", "map.c",
        // String / QSTR
        "qstr.c", "vstr.c", "unicode.c",
        // Compiler
        "lexer.c", "parse.c", "scope.c", "compile.c",
        "emitbc.c", "emitcommon.c", "emitglue.c", "emitnative.c", "emitndebug.c",
        // VM
        "vm.c", "bc.c", "runtime.c", "runtime_utils.c",
        // NLR — setjmp variant only (arch-independent)
        "nlr.c", "nlrsetjmp.c",
        // Objects
        "obj.c", "objarray.c", "objattrtuple.c", "objbool.c",
        "objboundmeth.c", "objcell.c", "objclosure.c", "objcomplex.c",
        "objdeque.c", "objdict.c", "objenumerate.c", "objexcept.c",
        "objfilter.c", "objfloat.c", "objfun.c", "objgenerator.c",
        "objgetitemiter.c", "objint.c", "objint_longlong.c",
        "objlist.c", "objmap.c", "objmodule.c", "objnamedtuple.c",
        "objnone.c", "objobject.c", "objpolyiter.c", "objproperty.c",
        "objrange.c", "objreversed.c", "objringio.c", "objset.c",
        "objsingleton.c", "objslice.c", "objstr.c", "objstringio.c",
        "objstrunicode.c", "objtuple.c", "objtype.c", "objzip.c",
        // Built-ins
        "argcheck.c", "binary.c", "builtinevex.c", "builtinhelp.c",
        "builtinimport.c", "modbuiltins.c",
        // Modules (with features gated by mpconfigport.h)
        "modarray.c", "modcmath.c", "modcollections.c",
        "modgc.c", "modmath.c", "modmicropython.c", "modstruct.c", "modsys.c",
        "modthread.c",
        // Misc
        "formatfloat.c", "frozenmod.c", "mpprint.c", "nativeglue.c",
        "opmethods.c", "pairheap.c", "parsenum.c", "parsenumbase.c",
        "persistentcode.c", "reader.c", "repl.c", "ringbuf.c",
        "scheduler.c", "sequence.c", "smallint.c", "stackctrl.c",
        "stream.c", "warning.c",
    ];
    for &src in py_sources {
        build.file(py.join(src));
    }

    // ── Step 4: shared/runtime helpers ───────────────────────────────────
    let shared_rt = vendor.join("shared/runtime");
    // Use the RV64 assembly GC helper instead of gchelper_generic.c;
    // the C version uses register-asm GNU extensions blocked by -std=c99.
    build.file(shared_rt.join("gchelper_rv64i.s"));
    build.file(shared_rt.join("pyexec.c"));
    build.file(shared_rt.join("interrupt_char.c"));
    // stdout_helpers.c omitted: it defines mp_hal_stdout_tx_strn_cooked which
    // conflicts with our mphalport.c implementation.

    // ── Step 5: embed port utility (gc_collect, nlr_jump_fail, mp_embed_*) ─
    // Only embed_util.c — NOT embed port's mphalport.c (uses stdio.h/printf).
    // Our src/c/vios/mphalport.c provides all HAL functions.
    build.file(embed_port.join("embed_util.c"));

    // ── Step 6: ViOS HAL + stubs + vnet module ───────────────────────────
    build.file(port_dir.join("mphalport.c"));
    // Stubs: readline, mp_lexer_new_from_file, mp_import_stat, disabled modules
    build.file(port_dir.join("vios_stubs.c"));
    // vnet Python module: TCP socket IPC via vios_net_* bridge (net_bridge.rs)
    build.file(port_dir.join("modvnet.c"));
    // GC register collector: gchelper_rv64i.s provides gc_helper_get_regs_and_sp;
    // gchelper_native.c wraps it into gc_helper_collect_regs_and_stack.
    build.file(shared_rt.join("gchelper_native.c"));

    build.compile("micropython");

    // ── Step 7: link against picolibc (setjmp, strchr, frexp, modf, ...) ───
    // The xPack riscv-none-elf-gcc toolchain bundles picolibc.  Dynamically
    // locate libc.a/libm.a/libgcc.a so they resolve undefined C symbols.
    link_picolibc_if_found();

    // Compiler_builtins (from Rust core) also provides memset/memcpy/malloc etc.
    // Allow duplicates so compiler_builtins wins for those and picolibc wins for
    // setjmp/longjmp/frexp/strchr which compiler_builtins doesn't provide.
    println!("cargo:rustc-link-arg=--allow-multiple-definition");

    // Linker script: place cell at VA 0x0E000000 (224 MB, above compositor)
    println!("cargo:rustc-link-arg=-Tcells/runtimes/micropython/micropython.ld");

    // Rerun triggers
    println!("cargo:rerun-if-changed=micropython.ld");
    println!("cargo:rerun-if-changed=src/c/vios/mpconfigport.h");
    println!("cargo:rerun-if-changed=src/c/vios/mphalport.c");
    println!("cargo:rerun-if-changed=src/c/vios/modvnet.c");
    println!("cargo:rerun-if-changed=src/c/vios/vios_stubs.c");
    println!("cargo:rerun-if-changed=gen_genhdr.py");
    println!("cargo:rerun-if-changed=vendor/py");
}

/// Locate picolibc/newlib from the riscv-none-elf-gcc toolchain and emit
/// the linker flags needed to resolve C runtime symbols (setjmp, strchr,
/// frexp, modf, etc.) that MicroPython depends on.
fn link_picolibc_if_found() {
    let libc_path = run_cross_compiler(&["--print-file-name=libc.a"]);
    let libgcc_path = run_cross_compiler(&["--print-libgcc-file-name"]);

    for p in [&libc_path, &libgcc_path] {
        if let Some(dir) = std::path::Path::new(p).parent() {
            let dir_str = dir.to_string_lossy();
            if !dir_str.is_empty() && dir_str != "." {
                println!("cargo:rustc-link-search=native={}", dir_str);
            }
        }
    }

    // Link picolibc + libgcc (provides setjmp/longjmp via gcc EH or libgcc).
    println!("cargo:rustc-link-lib=static=c");
    println!("cargo:rustc-link-lib=static=m");
    println!("cargo:rustc-link-lib=static=gcc");
}

fn run_cross_compiler(args: &[&str]) -> String {
    let mut all_args = vec!["-march=rv64gc", "-mabi=lp64d"];
    all_args.extend_from_slice(args);
    Command::new("riscv-none-elf-gcc")
        .args(&all_args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn find_python() -> String {
    // Expand `~` to the user home directory before probing.
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    let venv_python = format!(r"{}\.claude\skills\.venv\Scripts\python.exe", home);
    let candidates: &[&str] = &[venv_python.as_str(), "python3", "python"];
    for &c in candidates {
        if Command::new(c).arg("--version").output().is_ok() {
            return c.to_string();
        }
    }
    "python3".to_string()
}
