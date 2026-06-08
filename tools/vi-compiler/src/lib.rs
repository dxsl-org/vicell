//! vi-compiler — ViUI `.vi` DSL compiler.
//!
//! Pipeline: source text → [`lexer::tokenize`] → [`parser::parse`] → [`ast::ViFile`]
//!
//! P03: lexer + parser producing a structural AST.
//! P04: expression evaluator + Rust codegen from AST.

pub mod ast;
pub mod codegen;
pub mod error;
pub mod eval;
pub mod lexer;
pub mod parser;
pub mod token;

/// One-shot: tokenize + parse a `.vi` source string → AST.
pub fn compile_str(src: &str) -> Result<ast::ViFile, error::ParseError> {
    let tokens = lexer::tokenize(src)?;
    parser::parse(tokens)
}
