use std::{env, fs};
use vi_compiler::codegen::CodeGen;

fn main() {
    let path = match env::args().nth(1) {
        Some(p) => p,
        None    => { eprintln!("usage: vi-compiler <file.vi>"); std::process::exit(1); }
    };
    let src = match fs::read_to_string(&path) {
        Ok(s)  => s,
        Err(e) => { eprintln!("error reading {}: {}", path, e); std::process::exit(1); }
    };
    match vi_compiler::compile_str(&src) {
        Ok(file) => {
            let rust_src = CodeGen::new().generate(&file);
            println!("{}", rust_src);
        }
        Err(e) => { eprintln!("error: {}", e); std::process::exit(1); }
    }
}
