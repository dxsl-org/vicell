use std::prelude::v1::*;
use crate::error::LexError;
use crate::token::{Token, TokenKind, Span, keyword};

// ─── Lexer ───────────────────────────────────────────────────────────────────

struct Lexer<'src> {
    src:  &'src [u8],
    pos:  usize,
    line: u32,
    col:  u32,
}

impl<'src> Lexer<'src> {
    fn new(src: &'src str) -> Self {
        Self { src: src.as_bytes(), pos: 0, line: 1, col: 1 }
    }

    fn at_end(&self) -> bool { self.pos >= self.src.len() }
    fn peek(&self) -> u8 { if self.at_end() { 0 } else { self.src[self.pos] } }
    fn peek2(&self) -> u8 { if self.pos + 1 >= self.src.len() { 0 } else { self.src[self.pos + 1] } }

    fn advance(&mut self) -> u8 {
        let ch = self.src[self.pos];
        self.pos += 1;
        if ch == b'\n' { self.line += 1; self.col = 1; } else { self.col += 1; }
        ch
    }

    fn span_from(&self, start: usize, start_line: u32, start_col: u32) -> Span {
        Span::new(start_line, start_col, start as u32, (self.pos - start) as u32)
    }

    fn slice_str(&self, start: usize, end: usize) -> &str {
        std::str::from_utf8(&self.src[start..end]).unwrap_or("")
    }

    // ── Skip trivia ──────────────────────────────────────────────────────────

    fn skip_whitespace(&mut self) {
        while !self.at_end() && matches!(self.peek(), b' ' | b'\t' | b'\r' | b'\n') {
            self.advance();
        }
    }

    fn skip_line_comment(&mut self) {
        while !self.at_end() && self.peek() != b'\n' { self.advance(); }
    }

    fn skip_block_comment(&mut self, start: usize, sl: u32, sc: u32) -> Result<(), LexError> {
        loop {
            if self.at_end() {
                return Err(LexError::UnterminatedComment { span: Span::new(sl, sc, start as u32, 2) });
            }
            if self.peek() == b'*' && self.peek2() == b'/' {
                self.advance(); self.advance();
                return Ok(());
            }
            self.advance();
        }
    }

    // ── Literals ─────────────────────────────────────────────────────────────

    fn lex_string(&mut self, start: usize, sl: u32, sc: u32) -> Result<Token, LexError> {
        // pos is just after the opening `"`
        let content_start = self.pos;
        loop {
            if self.at_end() {
                return Err(LexError::UnterminatedString { span: Span::new(sl, sc, start as u32, 1) });
            }
            let ch = self.peek();
            if ch == b'\\' {
                self.advance(); // consume backslash
                if !self.at_end() { self.advance(); } // consume escaped char (includes '{')
            } else if ch == b'"' {
                break;
            } else {
                self.advance();
            }
        }
        let text = self.slice_str(content_start, self.pos).to_string();
        self.advance(); // consume closing `"`
        Ok(Token::new(TokenKind::StringLit, text, self.span_from(start, sl, sc)))
    }

    fn lex_color(&mut self, start: usize, sl: u32, sc: u32) -> Token {
        // pos is just after '#'
        while !self.at_end() && self.src[self.pos].is_ascii_hexdigit() {
            self.advance();
        }
        let text = self.slice_str(start, self.pos).to_string();
        Token::new(TokenKind::ColorLit, text, self.span_from(start, sl, sc))
    }

    fn lex_number(&mut self, start: usize, sl: u32, sc: u32) -> Token {
        // Consume digits
        while !self.at_end() && self.src[self.pos].is_ascii_digit() { self.advance(); }

        // Float?
        if !self.at_end() && self.src[self.pos] == b'.' && (self.pos + 1 < self.src.len()) && self.src[self.pos + 1].is_ascii_digit() {
            self.advance(); // consume '.'
            while !self.at_end() && self.src[self.pos].is_ascii_digit() { self.advance(); }
            let text = self.slice_str(start, self.pos).to_string();
            // check for unit suffix after float (e.g. 3.14em — unusual but handle)
            return if !self.at_end() && self.src[self.pos].is_ascii_alphabetic() {
                let _unit_start = self.pos;
                while !self.at_end() && self.src[self.pos].is_ascii_alphabetic() { self.advance(); }
                let full = self.slice_str(start, self.pos).to_string();
                Token::new(TokenKind::LengthLit, full, self.span_from(start, sl, sc))
            } else {
                Token::new(TokenKind::FloatLit, text, self.span_from(start, sl, sc))
            };
        }

        // Check for unit suffix: px, em, rem, pt, dp, vw, vh, ...
        if !self.at_end() && self.src[self.pos].is_ascii_alphabetic() {
            while !self.at_end() && self.src[self.pos].is_ascii_alphabetic() { self.advance(); }
            let text = self.slice_str(start, self.pos).to_string();
            return Token::new(TokenKind::LengthLit, text, self.span_from(start, sl, sc));
        }

        // Percent
        if !self.at_end() && self.src[self.pos] == b'%' {
            self.advance();
            let text = self.slice_str(start, self.pos).to_string();
            return Token::new(TokenKind::PercentLit, text, self.span_from(start, sl, sc));
        }

        let text = self.slice_str(start, self.pos).to_string();
        Token::new(TokenKind::IntLit, text, self.span_from(start, sl, sc))
    }

    fn lex_ident_or_keyword(&mut self, start: usize, sl: u32, sc: u32) -> Token {
        while !self.at_end() && (self.src[self.pos].is_ascii_alphanumeric() || self.src[self.pos] == b'_') {
            self.advance();
        }
        let text = self.slice_str(start, self.pos);
        let kind = keyword(text).unwrap_or(TokenKind::Ident);
        Token::new(kind, text, self.span_from(start, sl, sc))
    }

    // ── Main scan ────────────────────────────────────────────────────────────

    fn next_significant(&mut self) -> Result<Option<Token>, LexError> {
        loop {
            self.skip_whitespace();
            if self.at_end() { return Ok(None); }

            let sl = self.line;
            let sc = self.col;
            let start = self.pos;
            let ch = self.peek();

            // Comments
            if ch == b'/' && self.peek2() == b'/' {
                self.advance(); self.advance();
                self.skip_line_comment();
                continue;
            }
            if ch == b'/' && self.peek2() == b'*' {
                self.advance(); self.advance();
                self.skip_block_comment(start, sl, sc)?;
                continue;
            }

            // String literal
            if ch == b'"' {
                self.advance(); // consume opening quote
                return Ok(Some(self.lex_string(start, sl, sc)?));
            }

            // Color literal
            if ch == b'#' {
                self.advance(); // consume '#'
                return Ok(Some(self.lex_color(start, sl, sc)));
            }

            // Number
            if ch.is_ascii_digit() {
                self.advance();
                return Ok(Some(self.lex_number(start, sl, sc)));
            }

            // Identifier / keyword
            if ch.is_ascii_alphabetic() || ch == b'_' {
                self.advance();
                return Ok(Some(self.lex_ident_or_keyword(start, sl, sc)));
            }

            // Multi-char operators (check before single-char)
            self.advance();
            let tok = match ch {
                b'=' if self.peek() == b'>' => { self.advance(); Token::new(TokenKind::Arrow,   "=>", self.span_from(start, sl, sc)) }
                b'=' if self.peek() == b'=' => { self.advance(); Token::new(TokenKind::EqEq,    "==", self.span_from(start, sl, sc)) }
                b'!' if self.peek() == b'=' => { self.advance(); Token::new(TokenKind::BangEq,  "!=", self.span_from(start, sl, sc)) }
                b'<' if self.peek() == b'=' => { self.advance(); Token::new(TokenKind::LtEq,    "<=", self.span_from(start, sl, sc)) }
                b'>' if self.peek() == b'=' => { self.advance(); Token::new(TokenKind::GtEq,    ">=", self.span_from(start, sl, sc)) }
                b'&' if self.peek() == b'&' => { self.advance(); Token::new(TokenKind::And,     "&&", self.span_from(start, sl, sc)) }
                b'|' if self.peek() == b'|' => { self.advance(); Token::new(TokenKind::Or,      "||", self.span_from(start, sl, sc)) }
                b'+' if self.peek() == b'=' => { self.advance(); Token::new(TokenKind::PlusEq,  "+=", self.span_from(start, sl, sc)) }
                b'-' if self.peek() == b'=' => { self.advance(); Token::new(TokenKind::MinusEq, "-=", self.span_from(start, sl, sc)) }
                // Single-char
                b'+' => Token::new(TokenKind::Plus,      "+", self.span_from(start, sl, sc)),
                b'-' => Token::new(TokenKind::Minus,     "-", self.span_from(start, sl, sc)),
                b'*' => Token::new(TokenKind::Star,      "*", self.span_from(start, sl, sc)),
                b'/' => Token::new(TokenKind::Slash,     "/", self.span_from(start, sl, sc)),
                b'=' => Token::new(TokenKind::Assign,    "=", self.span_from(start, sl, sc)),
                b':' => Token::new(TokenKind::Colon,     ":", self.span_from(start, sl, sc)),
                b';' => Token::new(TokenKind::Semicolon, ";", self.span_from(start, sl, sc)),
                b',' => Token::new(TokenKind::Comma,     ",", self.span_from(start, sl, sc)),
                b'.' => Token::new(TokenKind::Dot,       ".", self.span_from(start, sl, sc)),
                b'!' => Token::new(TokenKind::Bang,      "!", self.span_from(start, sl, sc)),
                b'?' => Token::new(TokenKind::Question,  "?", self.span_from(start, sl, sc)),
                b'&' => Token::new(TokenKind::Ampersand, "&", self.span_from(start, sl, sc)),
                b'|' => Token::new(TokenKind::Pipe,      "|", self.span_from(start, sl, sc)),
                b'<' => Token::new(TokenKind::Lt,        "<", self.span_from(start, sl, sc)),
                b'>' => Token::new(TokenKind::Gt,        ">", self.span_from(start, sl, sc)),
                b'{' => Token::new(TokenKind::LBrace,    "{", self.span_from(start, sl, sc)),
                b'}' => Token::new(TokenKind::RBrace,    "}", self.span_from(start, sl, sc)),
                b'(' => Token::new(TokenKind::LParen,    "(", self.span_from(start, sl, sc)),
                b')' => Token::new(TokenKind::RParen,    ")", self.span_from(start, sl, sc)),
                b'[' => Token::new(TokenKind::LBracket,  "[", self.span_from(start, sl, sc)),
                b']' => Token::new(TokenKind::RBracket,  "]", self.span_from(start, sl, sc)),
                other => {
                    return Err(LexError::UnexpectedChar {
                        ch:   other as char,
                        span: self.span_from(start, sl, sc),
                    });
                }
            };
            return Ok(Some(tok));
        }
    }
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Tokenize `src`, skipping whitespace and comments.
/// Returns the token list terminated by `TokenKind::Eof`.
pub fn tokenize(src: &str) -> Result<Vec<Token>, LexError> {
    let mut lexer = Lexer::new(src);
    let mut tokens = Vec::new();
    while let Some(tok) = lexer.next_significant()? {
        tokens.push(tok);
    }
    let eof_span = Span::new(lexer.line, lexer.col, lexer.pos as u32, 0);
    tokens.push(Token::eof(eof_span));
    Ok(tokens)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use TokenKind::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        tokenize(src).unwrap()
            .into_iter()
            .filter(|t| t.kind != Eof)
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn keywords() {
        assert_eq!(kinds("component"), vec![KwComponent]);
        assert_eq!(kinds("property"),  vec![KwProperty]);
        assert_eq!(kinds("in"),        vec![KwIn]);
        assert_eq!(kinds("out"),       vec![KwOut]);
    }

    #[test]
    fn in_out_three_tokens() {
        // "in-out" lexes as KwIn Minus KwOut — parser combines into InOut visibility
        assert_eq!(kinds("in-out"), vec![KwIn, Minus, KwOut]);
    }

    #[test]
    fn length_literal() {
        assert_eq!(kinds("16px"), vec![LengthLit]);
        assert_eq!(kinds("8em"),  vec![LengthLit]);
        assert_eq!(kinds("2rem"), vec![LengthLit]);
    }

    #[test]
    fn color_literal() {
        assert_eq!(kinds("#ffffff"), vec![ColorLit]);
        assert_eq!(kinds("#fff"),    vec![ColorLit]);
    }

    #[test]
    fn string_literal_with_interpolation() {
        let toks = tokenize(r#""Count: \{count}""#).unwrap();
        assert_eq!(toks[0].kind, StringLit);
        assert!(toks[0].text.contains("\\{count}"), "raw escape must be preserved");
    }

    #[test]
    fn arrow_operator() {
        assert_eq!(kinds("=>"), vec![Arrow]);
        // Make sure single '=' is not eaten
        assert_eq!(kinds("= >"), vec![Assign, Gt]);
    }

    #[test]
    fn line_comment_skipped() {
        assert_eq!(kinds("// comment\ncomponent"), vec![KwComponent]);
    }

    #[test]
    fn block_comment_skipped() {
        assert_eq!(kinds("/* comment */ component"), vec![KwComponent]);
    }

    #[test]
    fn unterminated_string_error() {
        assert!(tokenize(r#""unterminated"#).is_err());
    }
}
