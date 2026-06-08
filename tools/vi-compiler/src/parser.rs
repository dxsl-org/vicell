use crate::ast::*;
use crate::error::ParseError;
use crate::token::{Span, Token, TokenKind};
use std::prelude::v1::*;

// ─── Parser ──────────────────────────────────────────────────────────────────

pub struct Parser {
    tokens: Vec<Token>,
    pos:    usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self { Self { tokens, pos: 0 } }

    // ── Token navigation ─────────────────────────────────────────────────────

    fn peek(&self) -> &Token          { &self.tokens[self.pos.min(self.tokens.len() - 1)] }
    fn peek_kind(&self) -> &TokenKind { &self.peek().kind }

    fn peek_at(&self, offset: usize) -> &Token {
        let idx = (self.pos + offset).min(self.tokens.len() - 1);
        &self.tokens[idx]
    }
    fn peek_kind_at(&self, offset: usize) -> &TokenKind { &self.peek_at(offset).kind }

    fn at_eof(&self) -> bool { *self.peek_kind() == TokenKind::Eof }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos.min(self.tokens.len() - 1)];
        if self.pos < self.tokens.len() - 1 { self.pos += 1; }
        tok
    }

    fn expect(&mut self, kind: TokenKind, context: &'static str) -> Result<Token, ParseError> {
        if *self.peek_kind() == kind {
            Ok(self.advance().clone())
        } else {
            Err(self.unexpected(context))
        }
    }

    fn unexpected(&self, expected: &'static str) -> ParseError {
        let tok = self.peek();
        ParseError::UnexpectedToken {
            got:      format!("{:?}({})", tok.kind, tok.text),
            expected,
            span:     tok.span,
        }
    }

    fn current_span(&self) -> Span { self.peek().span }

    // ── parse_file ───────────────────────────────────────────────────────────

    pub fn parse_file(&mut self) -> Result<ViFile, ParseError> {
        let mut imports    = Vec::new();
        let mut components = Vec::new();

        while !self.at_eof() {
            match self.peek_kind() {
                TokenKind::KwImport    => imports.push(self.parse_import()?),
                TokenKind::KwComponent => components.push(self.parse_component()?),
                _ => return Err(self.unexpected("import or component")),
            }
        }
        Ok(ViFile { imports, components })
    }

    // ── parse_import ─────────────────────────────────────────────────────────

    fn parse_import(&mut self) -> Result<Import, ParseError> {
        let span_start = self.current_span();
        self.expect(TokenKind::KwImport, "'import'")?;
        let path_tok = self.expect(TokenKind::StringLit, "file path string")?;
        self.expect(TokenKind::Semicolon, "';'")?;
        Ok(Import { path: path_tok.text, span: span_start })
    }

    // ── parse_component ──────────────────────────────────────────────────────

    fn parse_component(&mut self) -> Result<Component, ParseError> {
        let span_start = self.current_span();
        self.expect(TokenKind::KwComponent, "'component'")?;
        let name = self.expect(TokenKind::Ident, "component name")?.text;
        self.expect(TokenKind::LBrace, "'{'")?;

        let mut properties = Vec::new();
        let mut callbacks  = Vec::new();
        let mut children   = Vec::new();

        while !matches!(self.peek_kind(), TokenKind::RBrace | TokenKind::Eof) {
            // Property declaration: visibility? 'property' ...
            if self.is_visibility_or_property() {
                if *self.peek_kind() == TokenKind::KwCallback {
                    callbacks.push(self.parse_callback_decl()?);
                } else {
                    properties.push(self.parse_property_decl()?);
                }
            } else if *self.peek_kind() == TokenKind::Ident {
                // Must be a child element: IDENT {
                children.push(self.parse_element()?);
            } else {
                return Err(self.unexpected("property, callback, or element"));
            }
        }
        self.expect(TokenKind::RBrace, "'}'")?;
        Ok(Component { name, properties, callbacks, children, span: span_start })
    }

    fn is_visibility_or_property(&self) -> bool {
        matches!(
            self.peek_kind(),
            TokenKind::KwIn
            | TokenKind::KwOut
            | TokenKind::KwPrivate
            | TokenKind::KwProperty
            | TokenKind::KwCallback
        )
    }

    // ── parse_visibility ─────────────────────────────────────────────────────

    fn parse_visibility(&mut self) -> Option<Visibility> {
        match self.peek_kind() {
            TokenKind::KwIn => {
                self.advance();
                // in-out: KwIn Minus KwOut
                if *self.peek_kind() == TokenKind::Minus
                    && *self.peek_kind_at(1) == TokenKind::KwOut
                {
                    self.advance(); // consume '-'
                    self.advance(); // consume 'out'
                    Some(Visibility::InOut)
                } else {
                    Some(Visibility::In)
                }
            }
            TokenKind::KwOut     => { self.advance(); Some(Visibility::Out)     }
            TokenKind::KwPrivate => { self.advance(); Some(Visibility::Private) }
            _                    => None,
        }
    }

    // ── parse_property_decl ──────────────────────────────────────────────────

    fn parse_property_decl(&mut self) -> Result<PropertyDecl, ParseError> {
        let span_start = self.current_span();
        let visibility = self.parse_visibility();
        self.expect(TokenKind::KwProperty, "'property'")?;

        // '<' type '>'
        self.expect(TokenKind::Lt, "'<'")?;
        let ty = self.expect(TokenKind::Ident, "type name")?.text;
        self.expect(TokenKind::Gt, "'>'")?;

        let name = self.expect(TokenKind::Ident, "property name")?.text;

        let default = if *self.peek_kind() == TokenKind::Colon {
            self.advance(); // consume ':'
            Some(self.parse_expr_raw_until_semi())
        } else {
            None
        };
        self.expect(TokenKind::Semicolon, "';'")?;

        Ok(PropertyDecl { visibility, ty, name, default, span: span_start })
    }

    // ── parse_callback_decl ──────────────────────────────────────────────────

    fn parse_callback_decl(&mut self) -> Result<CallbackDecl, ParseError> {
        let span_start = self.current_span();
        self.expect(TokenKind::KwCallback, "'callback'")?;
        let name = self.expect(TokenKind::Ident, "callback name")?.text;
        let mut params = Vec::new();

        if *self.peek_kind() == TokenKind::LParen {
            self.advance();
            while !matches!(self.peek_kind(), TokenKind::RParen | TokenKind::Eof) {
                let pname = self.expect(TokenKind::Ident, "param name")?.text;
                self.expect(TokenKind::Colon, "':'")?;
                let pty = self.expect(TokenKind::Ident, "param type")?.text;
                params.push((pname, pty));
                if *self.peek_kind() == TokenKind::Comma { self.advance(); }
            }
            self.expect(TokenKind::RParen, "')'")?;
        }
        self.expect(TokenKind::Semicolon, "';'")?;
        Ok(CallbackDecl { name, params, span: span_start })
    }

    // ── parse_element ────────────────────────────────────────────────────────

    fn parse_element(&mut self) -> Result<Element, ParseError> {
        let span_start = self.current_span();
        let name = self.expect(TokenKind::Ident, "element name")?.text;
        self.expect(TokenKind::LBrace, "'{'")?;

        let mut bindings  = Vec::new();
        let mut callbacks = Vec::new();
        let mut children  = Vec::new();

        while !matches!(self.peek_kind(), TokenKind::RBrace | TokenKind::Eof) {
            match (self.peek_kind().clone(), self.peek_kind_at(1).clone()) {
                // callback binding: IDENT =>
                (TokenKind::Ident, TokenKind::Arrow) => {
                    callbacks.push(self.parse_callback_binding()?);
                }
                // binding: IDENT :
                (TokenKind::Ident, TokenKind::Colon) => {
                    bindings.push(self.parse_binding()?);
                }
                // child element: IDENT {
                (TokenKind::Ident, TokenKind::LBrace) => {
                    children.push(self.parse_element()?);
                }
                _ => return Err(self.unexpected("binding, callback, or child element")),
            }
        }
        self.expect(TokenKind::RBrace, "'}'")?;
        Ok(Element { name, bindings, callbacks, children, span: span_start })
    }

    // ── parse_binding ────────────────────────────────────────────────────────

    fn parse_binding(&mut self) -> Result<Binding, ParseError> {
        let span_start = self.current_span();
        let property = self.expect(TokenKind::Ident, "property name")?.text;
        self.expect(TokenKind::Colon, "':'")?;
        let value = self.parse_expr_raw_until_semi();
        self.expect(TokenKind::Semicolon, "';'")?;
        Ok(Binding { property, value, span: span_start })
    }

    // ── parse_callback_binding ───────────────────────────────────────────────

    fn parse_callback_binding(&mut self) -> Result<CallbackBinding, ParseError> {
        let span_start = self.current_span();
        let name = self.expect(TokenKind::Ident, "callback name")?.text;
        self.expect(TokenKind::Arrow, "'=>'")?;
        self.expect(TokenKind::LBrace, "'{'")?;
        let body = self.parse_raw_body();
        self.expect(TokenKind::RBrace, "'}'")?;
        // Optional trailing semicolon
        if *self.peek_kind() == TokenKind::Semicolon { self.advance(); }
        Ok(CallbackBinding { name, body, span: span_start })
    }

    // ── Expression helpers ───────────────────────────────────────────────────

    /// Collect tokens until ';' at brace-depth 0 (does NOT consume the ';').
    fn parse_expr_raw_until_semi(&mut self) -> Expr {
        let span_start = self.current_span();
        let mut parts  = Vec::<String>::new();
        let mut depth  = 0i32;
        loop {
            match self.peek_kind().clone() {
                TokenKind::LBrace | TokenKind::LParen | TokenKind::LBracket => {
                    depth += 1;
                    parts.push(self.advance().text.clone());
                }
                TokenKind::RBrace | TokenKind::RParen | TokenKind::RBracket => {
                    if depth == 0 { break; }
                    depth -= 1;
                    parts.push(self.advance().text.clone());
                }
                TokenKind::Semicolon if depth == 0 => break,
                TokenKind::Eof => break,
                // Preserve quotes so eval_binding can distinguish StringLit from Ident
                TokenKind::StringLit => parts.push(format!("\"{}\"", self.advance().text)),
                _ => parts.push(self.advance().text.clone()),
            }
        }
        Expr::Raw(RawExpr { text: parts.join(" "), span: span_start })
    }

    /// Collect tokens until the closing '}' at depth 0 (does NOT consume '}'.
    fn parse_raw_body(&mut self) -> String {
        let mut parts = Vec::<String>::new();
        let mut depth = 0i32;
        loop {
            match self.peek_kind().clone() {
                TokenKind::LBrace => { depth += 1; parts.push(self.advance().text.clone()); }
                TokenKind::RBrace => {
                    if depth == 0 { break; }
                    depth -= 1;
                    parts.push(self.advance().text.clone());
                }
                TokenKind::Eof => break,
                _ => parts.push(self.advance().text.clone()),
            }
        }
        parts.join(" ").trim().to_string()
    }
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Parse a token list (from `tokenize()`) into a `ViFile` AST.
pub fn parse(tokens: Vec<Token>) -> Result<ViFile, ParseError> {
    Parser::new(tokens).parse_file()
}
