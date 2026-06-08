//! Expression evaluator — converts `Expr::Raw` text into `TypedExpr` variants.
//!
//! Scope: the 7 expression forms used in counter.vi. No full Slint expression language.

use std::prelude::v1::*;
use crate::lexer::tokenize;
use crate::token::TokenKind;

// ─── Types ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum AugOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// Part of an interpolated string `"Count: \{count}"`.
#[derive(Debug, Clone, PartialEq)]
pub enum InterpolPart {
    /// Plain text segment.
    Literal(String),
    /// `\{var_name}` → variable name inside the braces.
    Var(String),
}

/// Typed expression — sufficient for emitting Rust code for counter.vi.
#[derive(Debug, Clone, PartialEq)]
pub enum TypedExpr {
    /// Integer literal: `0`, `42`
    IntLit(i64),
    /// String literal without interpolation: `"Increment"`
    StringLit(String),
    /// String with `\{var}` segments: `"Count: \{count}"`
    Interpolated(Vec<InterpolPart>),
    /// Color: `#ffffff` → r, g, b (each 0–255)
    ColorLit { r: u8, g: u8, b: u8 },
    /// Length with unit: `16px`, `8em` → numeric value only
    LengthLit(f32),
    /// Augmented assignment in callback body: `count += 1`
    AugAssign { name: String, op: AugOp, rhs: Box<TypedExpr> },
    /// Bare identifier or unrecognised expression — emitted as-is.
    Ident(String),
}

// ─── Public entry points ─────────────────────────────────────────────────────

/// Evaluate a property default expression given its declared type.
///
/// e.g. `eval_property("0", "int")` → `TypedExpr::IntLit(0)`
pub fn eval_property(raw: &str, ty_hint: &str) -> TypedExpr {
    let raw = raw.trim();
    match ty_hint {
        "int" => {
            if let Ok(n) = raw.parse::<i64>() {
                return TypedExpr::IntLit(n);
            }
        }
        "string" => {
            if let Some(s) = strip_quotes(raw) {
                return parse_string_content(s);
            }
        }
        "bool" => return TypedExpr::Ident(raw.to_string()),
        _ => {}
    }
    TypedExpr::Ident(raw.to_string())
}

/// Evaluate an element binding value, with context about which property and element it belongs to.
///
/// e.g. `eval_binding("16px", "padding", "VerticalLayout")` → `TypedExpr::LengthLit(16.0)`
pub fn eval_binding(raw: &str, _prop_name: &str, _elem_type: &str) -> TypedExpr {
    let raw = raw.trim();
    if raw.is_empty() { return TypedExpr::Ident(String::new()); }

    let first = raw.as_bytes()[0];

    // Color: #rrggbb or #rgb
    if first == b'#' {
        if let Some(c) = parse_color(&raw[1..]) { return c; }
    }

    // String literal (with or without interpolation)
    if first == b'"' {
        if let Some(s) = strip_quotes(raw) {
            return parse_string_content(s);
        }
    }

    // Number (possibly with unit suffix)
    if first.is_ascii_digit() || (first == b'-' && raw.len() > 1 && raw.as_bytes()[1].is_ascii_digit()) {
        return parse_number_or_length(raw);
    }

    TypedExpr::Ident(raw.to_string())
}

/// Evaluate a callback body — handles augmented assignment `count += 1`.
///
/// Uses the existing lexer to tokenize the body text.
pub fn eval_callback(raw: &str) -> TypedExpr {
    // Strip trailing semicolons / whitespace before tokenising
    let src = raw.trim().trim_end_matches(';').trim();
    let Ok(tokens) = tokenize(src) else {
        return TypedExpr::Ident(src.to_string());
    };

    // Filter out EOF
    let toks: Vec<_> = tokens.iter().filter(|t| t.kind != TokenKind::Eof).collect();

    // Pattern: Ident  AugOp  IntLit
    if toks.len() == 3
        && toks[0].kind == TokenKind::Ident
        && toks[2].kind == TokenKind::IntLit
    {
        let op = match toks[1].kind {
            TokenKind::PlusEq  => Some(AugOp::Add),
            TokenKind::MinusEq => Some(AugOp::Sub),
            // *=, /= not yet in lexer — fall through to Ident
            _ => None,
        };
        if let Some(op) = op {
            if let Ok(n) = toks[2].text.parse::<i64>() {
                return TypedExpr::AugAssign {
                    name: toks[0].text.clone(),
                    op,
                    rhs: Box::new(TypedExpr::IntLit(n)),
                };
            }
        }
    }

    TypedExpr::Ident(src.to_string())
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// Strip surrounding `"..."` and return the inner text, or `None` if not quoted.
fn strip_quotes(s: &str) -> Option<&str> {
    let b = s.as_bytes();
    if b.len() >= 2 && b[0] == b'"' && b[b.len() - 1] == b'"' {
        Some(&s[1..s.len() - 1])
    } else {
        None
    }
}

/// Parse string content (already unquoted) into `StringLit` or `Interpolated`.
///
/// Scans for `\{...}` segments; everything else is a literal segment.
fn parse_string_content(s: &str) -> TypedExpr {
    let bytes = s.as_bytes();
    let mut parts: Vec<InterpolPart> = Vec::new();
    let mut i = 0;
    let mut lit_start = 0;
    let mut has_interp = false;

    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'\\' && bytes[i + 1] == b'{' {
            // Flush accumulated literal
            if i > lit_start {
                parts.push(InterpolPart::Literal(s[lit_start..i].to_string()));
            }
            i += 2; // skip `\{`
            let var_start = i;
            // Collect until `}`
            while i < bytes.len() && bytes[i] != b'}' { i += 1; }
            parts.push(InterpolPart::Var(s[var_start..i].trim().to_string()));
            if i < bytes.len() { i += 1; } // skip `}`
            lit_start = i;
            has_interp = true;
        } else {
            i += 1;
        }
    }

    if !has_interp {
        return TypedExpr::StringLit(s.to_string());
    }

    // Flush trailing literal
    if lit_start < bytes.len() {
        parts.push(InterpolPart::Literal(s[lit_start..].to_string()));
    }

    TypedExpr::Interpolated(parts)
}

/// Parse a hex color string (`rrggbb` or `rgb`, without the leading `#`).
fn parse_color(hex: &str) -> Option<TypedExpr> {
    let hex = hex.trim();
    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(TypedExpr::ColorLit { r, g, b })
        }
        3 => {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
            Some(TypedExpr::ColorLit { r, g, b })
        }
        _ => None,
    }
}

/// Parse a number optionally followed by a unit suffix.
fn parse_number_or_length(s: &str) -> TypedExpr {
    // Find where digits (and optional decimal point / minus) end
    let mut end = 0;
    let bytes = s.as_bytes();
    if !bytes.is_empty() && bytes[0] == b'-' { end = 1; }
    while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b'.') { end += 1; }

    let num_str = &s[..end];
    let suffix  = s[end..].trim();

    if suffix.is_empty() {
        // Plain integer
        if let Ok(n) = num_str.parse::<i64>() { return TypedExpr::IntLit(n); }
        if let Ok(f) = num_str.parse::<f32>() { return TypedExpr::LengthLit(f); }
    } else {
        // Has unit suffix → LengthLit
        if let Ok(f) = num_str.parse::<f32>() { return TypedExpr::LengthLit(f); }
    }

    TypedExpr::Ident(s.to_string())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_property_default() {
        assert_eq!(eval_property("0", "int"),  TypedExpr::IntLit(0));
        assert_eq!(eval_property("42", "int"), TypedExpr::IntLit(42));
    }

    #[test]
    fn length_px() {
        assert_eq!(eval_binding("16px", "padding", "VerticalLayout"), TypedExpr::LengthLit(16.0));
    }

    #[test]
    fn length_em() {
        assert_eq!(eval_binding("8em", "spacing", "VerticalLayout"), TypedExpr::LengthLit(8.0));
    }

    #[test]
    fn color_rrggbb() {
        assert_eq!(
            eval_binding("#ffffff", "color", "Text"),
            TypedExpr::ColorLit { r: 255, g: 255, b: 255 }
        );
        assert_eq!(
            eval_binding("#1a2b3c", "color", "Text"),
            TypedExpr::ColorLit { r: 0x1a, g: 0x2b, b: 0x3c }
        );
    }

    #[test]
    fn color_rgb_shorthand() {
        assert_eq!(
            eval_binding("#fff", "color", "Text"),
            TypedExpr::ColorLit { r: 255, g: 255, b: 255 }
        );
    }

    #[test]
    fn string_with_interpolation() {
        let result = eval_binding(r#""Count: \{count}""#, "text", "Text");
        assert_eq!(result, TypedExpr::Interpolated(vec![
            InterpolPart::Literal("Count: ".into()),
            InterpolPart::Var("count".into()),
        ]));
    }

    #[test]
    fn string_without_interpolation() {
        let result = eval_binding(r#""Increment""#, "text", "Button");
        assert_eq!(result, TypedExpr::StringLit("Increment".into()));
    }

    #[test]
    fn callback_augmented_assignment() {
        let result = eval_callback("count += 1 ;");
        assert_eq!(result, TypedExpr::AugAssign {
            name: "count".into(),
            op:   AugOp::Add,
            rhs:  Box::new(TypedExpr::IntLit(1)),
        });
    }

    #[test]
    fn unknown_expr_falls_back_to_ident() {
        let result = eval_binding("someIdent", "x", "Widget");
        assert_eq!(result, TypedExpr::Ident("someIdent".into()));
    }
}
