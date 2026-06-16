use crate::token::Span;
use std::fmt;

// ─── LexError ────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum LexError {
    UnexpectedChar { ch: char, span: Span },
    UnterminatedString { span: Span },
    UnterminatedComment { span: Span },
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedChar { ch, span } =>
                write!(f, "{}:{}: unexpected character '{}'", span.line, span.col, ch),
            Self::UnterminatedString { span } =>
                write!(f, "{}:{}: unterminated string literal", span.line, span.col),
            Self::UnterminatedComment { span } =>
                write!(f, "{}:{}: unterminated block comment", span.line, span.col),
        }
    }
}

impl std::error::Error for LexError {}

// ─── ParseError ──────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ParseError {
    UnexpectedToken { got: String, expected: &'static str, span: Span },
    UnexpectedEof   { expected: &'static str },
    Lex(LexError),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedToken { got, expected, span } =>
                write!(f, "{}:{}: expected {} but got '{}'", span.line, span.col, expected, got),
            Self::UnexpectedEof { expected } =>
                write!(f, "unexpected end of file, expected {}", expected),
            Self::Lex(e) => write!(f, "lex error: {}", e),
        }
    }
}

impl std::error::Error for ParseError {}

impl From<LexError> for ParseError {
    fn from(e: LexError) -> Self { Self::Lex(e) }
}
