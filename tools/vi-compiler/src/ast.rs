use std::prelude::v1::*;
use crate::token::Span;

// ─── Top-level ───────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct ViFile {
    pub imports:    Vec<Import>,
    pub components: Vec<Component>,
}

#[derive(Debug)]
pub struct Import {
    pub path: String,
    pub span: Span,
}

// ─── Component ───────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct Component {
    pub name:       String,
    pub properties: Vec<PropertyDecl>,
    pub callbacks:  Vec<CallbackDecl>,
    pub children:   Vec<Child>,
    pub span:       Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Visibility {
    In,
    Out,
    InOut,
    Private,
}

#[derive(Debug)]
pub struct PropertyDecl {
    pub visibility: Option<Visibility>,
    pub ty:         String,
    pub name:       String,
    pub default:    Option<Expr>,
    pub span:       Span,
}

#[derive(Debug)]
pub struct CallbackDecl {
    pub name:   String,
    pub params: Vec<(String, String)>, // (param_name, type_name)
    pub span:   Span,
}

// ─── Element ─────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct Element {
    pub name:      String,
    pub bindings:  Vec<Binding>,
    pub callbacks: Vec<CallbackBinding>,
    pub children:  Vec<Child>,
    pub span:      Span,
}

/// A child within a component or element body — either a concrete element
/// or a control-flow construct (`if` / `for`).
#[derive(Debug)]
pub enum Child {
    /// A concrete widget element with bindings and children.
    Element(Element),
    /// Conditional rendering: `if condition { ... }`
    If {
        /// Raw condition expression (may contain `self.X` property refs).
        cond: String,
        body: Vec<Child>,
        span: Span,
    },
    /// Loop rendering: `for var in iter { ... }`
    For {
        var:  String,
        iter: String,
        body: Vec<Child>,
        span: Span,
    },
}

#[derive(Debug)]
pub struct Binding {
    pub property: String,
    pub value:    Expr,
    pub span:     Span,
}

#[derive(Debug)]
pub struct CallbackBinding {
    pub name: String,
    pub body: String, // raw source text between '{ ' and ' }'
    pub span: Span,
}

// ─── Expressions ─────────────────────────────────────────────────────────────

/// P03: all expressions are raw source text.
/// P04 will extend this with typed variants.
#[derive(Debug)]
pub struct RawExpr {
    pub text: String,
    pub span: Span,
}

#[derive(Debug)]
pub enum Expr {
    Raw(RawExpr),
    // P04: Literal(Literal), Ident(String), BinOp(...), Ternary(...), Interpolated(...)
}
