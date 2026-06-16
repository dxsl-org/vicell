use std::prelude::v1::*;

/// Source location for error messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub line:  u32,  // 1-based
    pub col:   u32,  // 1-based
    pub start: u32,  // byte offset from source start
    pub len:   u32,
}

impl Span {
    pub fn new(line: u32, col: u32, start: u32, len: u32) -> Self {
        Self { line, col, start, len }
    }
}

// ─── TokenKind ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    // ── Structural keywords ───────────────────────────────────────────────
    KwComponent,    // component
    KwProperty,     // property
    KwImport,       // import
    KwExport,       // export
    KwIn,           // in
    KwOut,          // out
    KwPrivate,      // private
    KwCallback,     // callback
    KwIf,           // if
    KwElse,         // else
    KwFor,          // for
    KwReturn,       // return
    KwAnimate,      // animate (parsed, rejected by parser until G3)
    KwTrue,         // true
    KwFalse,        // false

    // ── Identifiers ───────────────────────────────────────────────────────
    Ident,

    // ── Literals ─────────────────────────────────────────────────────────
    IntLit,         // 42
    FloatLit,       // 3.14
    StringLit,      // "text" (raw content, \{} preserved as-is)
    ColorLit,       // #ffffff, #fff, #ffffffff
    LengthLit,      // 16px, 8em, 2rem, 4pt
    PercentLit,     // 50%

    // ── Multi-char operators ──────────────────────────────────────────────
    Arrow,          // =>
    EqEq,           // ==
    BangEq,         // !=
    LtEq,           // <=
    GtEq,           // >=
    And,            // &&
    Or,             // ||
    PlusEq,         // +=
    MinusEq,        // -=

    // ── Single-char operators/punctuation ────────────────────────────────
    Plus,           // +
    Minus,          // -
    Star,           // *
    Slash,          // /
    Assign,         // =
    Colon,          // :
    Semicolon,      // ;
    Comma,          // ,
    Dot,            // .
    Bang,           // !
    Question,       // ?
    Ampersand,      // &
    Pipe,           // |
    Lt,             // <
    Gt,             // >

    // ── Brackets ─────────────────────────────────────────────────────────
    LBrace,         // {
    RBrace,         // }
    LParen,         // (
    RParen,         // )
    LBracket,       // [
    RBracket,       // ]

    Eof,
}

// ─── Token ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub text: String,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, text: impl Into<String>, span: Span) -> Self {
        Self { kind, text: text.into(), span }
    }

    pub fn eof(span: Span) -> Self {
        Self { kind: TokenKind::Eof, text: String::new(), span }
    }
}

// ─── Keyword map ─────────────────────────────────────────────────────────────

pub fn keyword(s: &str) -> Option<TokenKind> {
    match s {
        "component" => Some(TokenKind::KwComponent),
        "property"  => Some(TokenKind::KwProperty),
        "import"    => Some(TokenKind::KwImport),
        "export"    => Some(TokenKind::KwExport),
        "in"        => Some(TokenKind::KwIn),
        "out"       => Some(TokenKind::KwOut),
        "private"   => Some(TokenKind::KwPrivate),
        "callback"  => Some(TokenKind::KwCallback),
        "if"        => Some(TokenKind::KwIf),
        "else"      => Some(TokenKind::KwElse),
        "for"       => Some(TokenKind::KwFor),
        "return"    => Some(TokenKind::KwReturn),
        "animate"   => Some(TokenKind::KwAnimate),
        "true"      => Some(TokenKind::KwTrue),
        "false"     => Some(TokenKind::KwFalse),
        _           => None,
    }
}
