# Phase 01 — Expression Evaluator

**Plan**: [plan.md](plan.md)
**Status**: ⬜ Planned
**File**: `tools/vi-compiler/src/eval.rs`

---

## Overview

Parse `Expr::Raw(text)` into `TypedExpr` — typed expression variants the codegen can emit Rust for.

Scope: exactly the 7 expression forms used in `counter.vi`. No full expression language.

---

## TypedExpr Enum

```rust
pub enum TypedExpr {
    /// Integer literal: `0`, `42`
    IntLit(i64),
    /// String literal without interpolation: `"Increment"`
    StringLit(String),
    /// String with \{var} interpolation: `"Count: \{count}"`
    Interpolated(Vec<InterpolPart>),
    /// Color: `#ffffff` → r, g, b
    ColorLit { r: u8, g: u8, b: u8 },
    /// Length: `16px`, `8em` → f32 (strip unit)
    LengthLit(f32),
    /// Augmented assignment in callbacks: `count += 1`
    AugAssign { name: String, op: AugOp, rhs: Box<TypedExpr> },
    /// Bare identifier (fallback for unknown expressions)
    Ident(String),
}

pub enum InterpolPart {
    Literal(String),   // plain text segment
    Var(String),       // \{var_name} → variable name
}

pub enum AugOp { Add, Sub, Mul, Div }
```

---

## Public Functions

```rust
/// Evaluate a property default expression with a type hint.
/// e.g. eval_property("0", "int") → TypedExpr::IntLit(0)
pub fn eval_property(raw: &str, ty_hint: &str) -> TypedExpr

/// Evaluate an element binding value.
/// e.g. eval_binding("16px", "padding", "VerticalLayout") → LengthLit(16.0)
pub fn eval_binding(raw: &str, prop_name: &str, elem_type: &str) -> TypedExpr

/// Evaluate a callback body (augmented assignment or raw expression).
/// e.g. eval_callback("count += 1 ;") → AugAssign { name: "count", op: Add, rhs: IntLit(1) }
pub fn eval_callback(raw: &str) -> TypedExpr
```

---

## Evaluation Rules

### `eval_property(raw, ty_hint)`

| ty_hint | raw | Result |
|---------|-----|--------|
| `"int"` | `"0"`, `"42"` | `IntLit(n)` |
| `"float"` | `"3.14"` | Not in counter.vi — emit `Ident(raw)` fallback |
| `"string"` | `"\"text\""` | `StringLit("text")` |
| `"bool"` | `"true"` | `Ident("true")` |
| anything | else | `Ident(raw.trim())` |

### `eval_binding(raw, prop_name, elem_type)`

Detection order (inspect first char of trimmed raw):
1. Starts with `"#"` → `ColorLit { r, g, b }` (parse hex `#rrggbb` or `#rgb`)
2. Starts with `'"'` → check for `\{` inside → `Interpolated` or `StringLit`
3. Digit or `-` digit → check for unit suffix (`px`, `em`, `rem`, `dp`, `pt`, `vw`, `vh`) → `LengthLit(f)` or `IntLit(n)` or `Ident`
4. else → `Ident(raw.trim())`

**String interpolation parsing** (`"Count: \{count}"`):
- Scan byte-by-byte through string content (after stripping surrounding `"`)
- Accumulate literal segment; on `\{` → push current literal as `InterpolPart::Literal`, collect until `}` as `InterpolPart::Var`
- If no `\{` found → `StringLit`; else → `Interpolated`

**Color parsing** (`#ffffff`, `#fff`):
- 6-char: `r = hex(1..3)`, `g = hex(3..5)`, `b = hex(5..7)`
- 3-char: expand each nibble: `r = n*17`, `g = n*17`, `b = n*17`
- Invalid hex → `Ident(raw.trim())` fallback

**Length parsing** (`16px`, `8em`):
- Strip trailing alphabetic suffix
- Parse remaining as `f32`
- Return `LengthLit(value)`

### `eval_callback(raw)`

Reuse existing `tokenize()` from `crate::lexer`:
1. Tokenize the raw body (strip trailing `;` first)
2. Match pattern: `[Ident(name), PlusEq|MinusEq|StarEq|SlashEq, IntLit(n)]`
3. Map token kind → `AugOp`; parse `IntLit` text → `i64`
4. Return `AugAssign { name, op, rhs: Box::new(IntLit(n)) }`
5. No match → `Ident(raw.trim())` fallback

---

## Unit Tests (in `eval.rs`)

```rust
#[cfg(test)]
mod tests {
    // 1. int property default
    // 2. length binding px
    // 3. length binding em
    // 4. color #rrggbb
    // 5. color #rgb
    // 6. string with interpolation
    // 7. string without interpolation
    // 8. callback augmented assignment (+=)
    // 9. fallback to Ident for unknown
}
```

---

## Implementation Steps

1. Create `tools/vi-compiler/src/eval.rs`
   - Define `TypedExpr`, `InterpolPart`, `AugOp` enums (with `#[derive(Debug, Clone, PartialEq)]`)
   - Implement `parse_color()`, `parse_length()`, `parse_string_content()` helpers
   - Implement `eval_property()`, `eval_binding()`, `eval_callback()` per rules above
   - Add 9 unit tests

2. Update `tools/vi-compiler/src/lib.rs`
   - Add `pub mod eval;`

3. Run `cargo test --manifest-path tools/vi-compiler/Cargo.toml -- eval`
   - All 9 tests must pass

---

## Success Criteria

- [ ] `TypedExpr` covers all 7 variants needed for counter.vi
- [ ] `eval_callback("count += 1 ;")` → `AugAssign { name: "count", op: Add, rhs: IntLit(1) }`
- [ ] `eval_binding("Count: \\{count}", "text", "Text")` → `Interpolated([Literal("Count: "), Var("count")])`
- [ ] `eval_binding("#ffffff", "color", "Text")` → `ColorLit { r: 255, g: 255, b: 255 }`
- [ ] All 9 unit tests pass
- [ ] `cargo clippy` clean
