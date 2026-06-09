use vi_compiler::{codegen::CodeGen, compile_str};

// ─── New widget registry tests ───────────────────────────────────────────────

#[test]
fn test_progressbar_codegen() {
    let src = r#"
component SensorPanel {
    in property <float> battery: 0.0;
    VBox {
        ProgressBar { value: self.battery; }
    }
}
"#;
    let file = compile_str(src).expect("parse failed");
    let rust = CodeGen::new().generate(&file);
    assert!(
        rust.contains("ProgressBar::new("),
        "ProgressBar::new missing: {}",
        &rust[..rust.len().min(600)]
    );
    assert!(
        rust.contains("battery"),
        "battery signal ref missing: {}",
        &rust[..rust.len().min(600)]
    );
}

#[test]
fn test_slider_codegen() {
    let src = r#"
component Controls {
    in property <float> speed: 0.5;
    VBox {
        Slider { value: self.speed; }
    }
}
"#;
    let file = compile_str(src).expect("parse failed");
    let rust = CodeGen::new().generate(&file);
    assert!(
        rust.contains("Slider::new("),
        "Slider::new missing: {}",
        &rust[..rust.len().min(600)]
    );
    assert!(
        rust.contains("speed"),
        "speed signal ref missing: {}",
        &rust[..rust.len().min(600)]
    );
}

#[test]
fn test_listview_codegen() {
    let src = r#"
component Log {
    in property <string> entries: "";
    VBox {
        ListView { items: self.entries; }
    }
}
"#;
    let file = compile_str(src).expect("parse failed");
    let rust = CodeGen::new().generate(&file);
    assert!(
        rust.contains("ListView::new("),
        "ListView::new missing: {}",
        &rust[..rust.len().min(600)]
    );
    assert!(
        rust.contains("entries"),
        "entries signal ref missing: {}",
        &rust[..rust.len().min(600)]
    );
}

#[test]
fn test_unknown_widget_emits_compile_error() {
    let src = r#"
component Broken {
    VBox {
        FooWidget { }
    }
}
"#;
    let file = compile_str(src).expect("parse failed");
    let rust = CodeGen::new().generate(&file);
    assert!(
        rust.contains("compile_error!"),
        "compile_error! not emitted for unknown widget: {}",
        &rust[..rust.len().min(600)]
    );
    assert!(
        rust.contains("FooWidget"),
        "widget name not in error msg: {}",
        &rust[..rust.len().min(600)]
    );
}

fn gen_counter() -> String {
    let src = include_str!("fixtures/counter.vi");
    let file = compile_str(src).expect("counter.vi should parse");
    CodeGen::new().generate(&file)
}

#[test]
fn counter_emits_struct_and_impl() {
    let out = gen_counter();
    assert!(out.contains("pub struct Counter"),     "missing struct Counter");
    assert!(out.contains("impl Counter"),            "missing impl Counter");
    assert!(out.contains("pub fn build()"),          "missing build() fn");
}

#[test]
fn counter_emits_signal_new() {
    let out = gen_counter();
    assert!(out.contains("Signal::new(0i32)"), "missing Signal::new(0i32) for count property");
}

#[test]
fn counter_emits_label_new() {
    let out = gen_counter();
    assert!(out.contains("Label::new("), "missing Label::new(...)");
}

#[test]
fn counter_emits_button_new() {
    let out = gen_counter();
    assert!(out.contains("Button::new("), "missing Button::new(...)");
}

#[test]
fn counter_emits_column_new() {
    let out = gen_counter();
    assert!(out.contains("Column::new("), "missing Column::new(...)");
}

#[test]
fn counter_emits_into_parts() {
    let out = gen_counter();
    assert!(out.contains(".into_parts()"), "missing .into_parts() for computed text");
}

#[test]
fn counter_emits_update_closure() {
    let out = gen_counter();
    assert!(out.contains(".update(|n|"), "missing .update(|n| ...) in button callback");
}

#[test]
fn counter_emits_subscription_handle_field() {
    let out = gen_counter();
    assert!(out.contains("SubscriptionHandle"), "missing SubscriptionHandle field");
}

#[test]
fn counter_has_pub_count_field() {
    let out = gen_counter();
    assert!(out.contains("pub count: Signal<i32>"), "missing pub count: Signal<i32>");
}

#[test]
fn test_if_codegen() {
    let src = r#"
component ShowPanel {
    in property <bool> show_panel: true;
    VBox {
        if self.show_panel {
            Label { text: "Panel visible"; }
        }
    }
}
"#;
    let file = vi_compiler::compile_str(src).expect("parse failed");
    let mut gen = vi_compiler::codegen::CodeGen::new();
    let rust = gen.generate(&file);
    // build() is static — properties are local variables, not self fields.
    assert!(
        rust.contains("if *show_panel.get()"),
        "if desugaring missing (expected local var, not self.): {}",
        &rust[..rust.len().min(500)]
    );
    assert!(
        rust.contains("alloc::vec![]"),
        "empty else branch missing: {}",
        &rust[..rust.len().min(500)]
    );
    // Bool property default `true` must emit Signal::new(true) not Signal::new(String::from(true)).
    assert!(
        rust.contains("Signal::new(true)"),
        "bool default not emitted correctly: {}",
        &rust[..rust.len().min(500)]
    );
}

#[test]
fn test_for_codegen() {
    let src = r#"
component ItemList {
    in property <string> items: "";
    VBox {
        for item in self.items {
            Label { text: item; }
        }
    }
}
"#;
    let file = vi_compiler::compile_str(src).expect("parse failed");
    let mut gen = vi_compiler::codegen::CodeGen::new();
    let rust = gen.generate(&file);
    assert!(rust.contains("iter()"),      "for loop iter() missing: {}", &rust[..rust.len().min(500)]);
    assert!(rust.contains("enumerate()"), "enumerate missing: {}",        &rust[..rust.len().min(500)]);
    assert!(rust.contains("Column::new"), "Column wrapper missing: {}",   &rust[..rust.len().min(500)]);
    // build() is static — iter expression is a local variable ref, not self.
    assert!(
        rust.contains("*items.get()"),
        "for iter should desugar to local var (no self.): {}",
        &rust[..rust.len().min(500)]
    );
}

#[test]
fn test_desugar_prop_refs() {
    // Test the desugar helper indirectly via a component that uses self.X in a condition
    let src = r#"
component Cond {
    in property <bool> flag: false;
    VBox {
        if self.flag {
            Label { text: "yes"; }
        }
    }
}
"#;
    let file = vi_compiler::compile_str(src).expect("parse failed");
    let mut gen = vi_compiler::codegen::CodeGen::new();
    let rust = gen.generate(&file);
    // build() is static — `self.flag` in .vi source desugars to local variable `*flag.get()`.
    assert!(
        rust.contains("*flag.get()"),
        "flag not desugared (expected local var, not self.): {}",
        &rust[..rust.len().min(500)]
    );
}

// ─── Phase 01: typed Expr + compile_expr tests ──────────────────────────────

/// Bool property default `true` parses as Literal::Bool → Signal::new(true).
#[test]
fn test_bool_property_typed_default() {
    let src = r#"
component Flags {
    in property <bool> flag: true;
    VBox { }
}
"#;
    let file = vi_compiler::compile_str(src).expect("parse failed");
    let rust = CodeGen::new().generate(&file);
    assert!(
        rust.contains("Signal::new(true)"),
        "bool default 'true' should emit Signal::new(true), not String wrapping: {}",
        &rust[..rust.len().min(600)]
    );
    // Must NOT emit Signal::new(String::from("true"))
    assert!(
        !rust.contains("Signal::new(String::from(\"true\"))"),
        "bool should not be wrapped in String::from: {}",
        &rust[..rust.len().min(600)]
    );
}

/// Binding `value: self.speed` produces a `.clone()` of the local signal var.
#[test]
fn test_self_prop_binding_clones_signal() {
    let src = r#"
component Controls {
    in property <float> speed: 0.5;
    VBox {
        Slider { value: self.speed; }
    }
}
"#;
    let file = vi_compiler::compile_str(src).expect("parse failed");
    let rust = CodeGen::new().generate(&file);
    // self.speed parsed as SelfProp("speed") → find_signal_binding emits speed.clone()
    assert!(
        rust.contains("speed.clone()"),
        "self.speed binding should emit speed.clone(): {}",
        &rust[..rust.len().min(600)]
    );
}

/// String interpolation `"Count: \{count}"` produces `.into_parts()` map chain.
#[test]
fn test_interpolated_text_into_parts() {
    let out = gen_counter();
    assert!(
        out.contains(".into_parts()"),
        "interpolated text must use .into_parts(): {}",
        &out[..out.len().min(600)]
    );
    assert!(
        out.contains(".map("),
        "interpolated text must use .map(): {}",
        &out[..out.len().min(600)]
    );
}

// ─── Syntax-validity (syn) ───────────────────────────────────────────────────

/// The counter.vi fixture must produce syntactically well-formed Rust.
///
/// `syn::parse_str::<syn::File>` is a fast, zero-linking check that catches
/// malformed token output (unbalanced braces, bad expression structure,
/// stray punctuation). It does NOT perform type or path resolution — a
/// call like `Signal::new(String::from(true))` would pass syn because it is
/// syntactically valid, even though it fails `rustc`. For type correctness
/// see the per-property-type tests in this file (e.g. `test_if_codegen`).
#[test]
fn counter_generates_valid_rust_syntax() {
    let rust = gen_counter();
    syn::parse_str::<syn::File>(&rust)
        .unwrap_or_else(|e| panic!("counter.vi codegen produced invalid Rust: {e}\n--- generated ---\n{rust}"));
}

/// A component with all primitive property types must produce valid Rust.
#[test]
fn all_property_types_generate_valid_rust_syntax() {
    let src = r#"
component AllTypes {
    in property <bool>   flag:  false;
    in property <int>    count: 0;
    in property <float>  ratio: 1.0;
    in property <string> label: "hi";
    VBox { }
}
"#;
    let file = vi_compiler::compile_str(src).expect("parse failed");
    let rust = vi_compiler::codegen::CodeGen::new().generate(&file);
    syn::parse_str::<syn::File>(&rust)
        .unwrap_or_else(|e| panic!("all-types codegen invalid Rust: {e}\n--- generated ---\n{rust}"));
}

// ─── Missing widget codegen tests ────────────────────────────────────────────

#[test]
fn test_flexbox_codegen() {
    let src = r#"
component Flex {
    HFlex {
        Label { text: "a"; }
        Label { text: "b"; }
    }
}
"#;
    let file = vi_compiler::compile_str(src).expect("parse failed");
    let rust = vi_compiler::codegen::CodeGen::new().generate(&file);
    assert!(rust.contains("FlexBox::"), "FlexBox ctor missing: {}", &rust[..rust.len().min(600)]);
}

#[test]
fn test_vflex_codegen() {
    let src = r#"
component VFlexComp {
    VFlex {
        Label { text: "x"; }
    }
}
"#;
    let file = vi_compiler::compile_str(src).expect("parse failed");
    let rust = vi_compiler::codegen::CodeGen::new().generate(&file);
    assert!(rust.contains("FlexBox::"), "VFlex → FlexBox ctor missing: {}", &rust[..rust.len().min(600)]);
}

#[test]
fn test_checkbox_codegen() {
    let src = r#"
component Check {
    in property <bool> checked: false;
    VBox {
        CheckBox { checked: self.checked; }
    }
}
"#;
    let file = vi_compiler::compile_str(src).expect("parse failed");
    let rust = vi_compiler::codegen::CodeGen::new().generate(&file);
    assert!(rust.contains("CheckBox::new("), "CheckBox::new missing: {}", &rust[..rust.len().min(600)]);
    assert!(rust.contains("checked"),        "checked signal missing: {}",  &rust[..rust.len().min(600)]);
}

#[test]
fn test_touch_area_codegen() {
    let src = r#"
component Tap {
    VBox {
        TouchArea { }
    }
}
"#;
    let file = vi_compiler::compile_str(src).expect("parse failed");
    let rust = vi_compiler::codegen::CodeGen::new().generate(&file);
    assert!(rust.contains("TouchArea::new("), "TouchArea::new missing: {}", &rust[..rust.len().min(600)]);
}

#[test]
fn test_card_codegen() {
    let src = r#"
component C {
    VBox {
        Card { Label { text: "hi"; } }
    }
}
"#;
    let file = vi_compiler::compile_str(src).expect("parse failed");
    let rust = vi_compiler::codegen::CodeGen::new().generate(&file);
    assert!(rust.contains("Card::"), "Card ctor missing: {}", &rust[..rust.len().min(600)]);
}

#[test]
fn test_divider_codegen() {
    let src = r#"
component D {
    VBox {
        Divider { }
    }
}
"#;
    let file = vi_compiler::compile_str(src).expect("parse failed");
    let rust = vi_compiler::codegen::CodeGen::new().generate(&file);
    assert!(rust.contains("Divider::horizontal("), "Divider::horizontal missing: {}", &rust[..rust.len().min(600)]);
}

#[test]
fn test_space_codegen() {
    let src = r#"
component S {
    VBox {
        Space { }
    }
}
"#;
    let file = vi_compiler::compile_str(src).expect("parse failed");
    let rust = vi_compiler::codegen::CodeGen::new().generate(&file);
    assert!(rust.contains("Space::new("), "Space::new missing: {}", &rust[..rust.len().min(600)]);
}

#[test]
fn test_image_codegen() {
    let src = r#"
component Img {
    VBox {
        Image { }
    }
}
"#;
    let file = vi_compiler::compile_str(src).expect("parse failed");
    let rust = vi_compiler::codegen::CodeGen::new().generate(&file);
    assert!(rust.contains("Image::new("), "Image::new missing: {}", &rust[..rust.len().min(600)]);
}

#[test]
fn test_scroll_area_codegen() {
    let src = r#"
component Scroll {
    VBox {
        ScrollArea { Label { text: "content"; } }
    }
}
"#;
    let file = vi_compiler::compile_str(src).expect("parse failed");
    let rust = vi_compiler::codegen::CodeGen::new().generate(&file);
    assert!(rust.contains("ScrollArea::"), "ScrollArea ctor missing: {}", &rust[..rust.len().min(600)]);
}

/// compile_expr unit tests — typed AST nodes → Rust source strings.
#[cfg(test)]
mod compile_expr_tests {
    use vi_compiler::ast::{BinOpKind, Expr, InterpPart, Literal, RawExpr, UnaryOp};
    use vi_compiler::eval::{compile_expr, ExprCtx};
    use vi_compiler::token::Span;

    fn build_fn() -> ExprCtx { ExprCtx::BuildFn }
    fn reactive() -> ExprCtx { ExprCtx::Reactive }

    #[test]
    fn bool_literal_true() {
        let e = Expr::Literal(Literal::Bool(true));
        assert_eq!(compile_expr(&e, build_fn()), "true");
    }

    #[test]
    fn bool_literal_false() {
        let e = Expr::Literal(Literal::Bool(false));
        assert_eq!(compile_expr(&e, build_fn()), "false");
    }

    #[test]
    fn int_literal() {
        let e = Expr::Literal(Literal::Int(42));
        assert_eq!(compile_expr(&e, build_fn()), "42");
    }

    #[test]
    fn float_literal() {
        let e = Expr::Literal(Literal::Float(3.14));
        assert_eq!(compile_expr(&e, build_fn()), "3.14_f32");
    }

    #[test]
    fn str_literal() {
        let e = Expr::Literal(Literal::Str("hello".to_string()));
        assert_eq!(compile_expr(&e, build_fn()), "\"hello\"");
    }

    #[test]
    fn self_prop_build_fn_context() {
        // In build() context: SelfProp("speed") → "*speed.get()"
        let e = Expr::SelfProp("speed".to_string());
        assert_eq!(compile_expr(&e, build_fn()), "*speed.get()");
    }

    #[test]
    fn self_prop_reactive_context() {
        // In reactive (map) context: SelfProp("speed") → "speed"
        let e = Expr::SelfProp("speed".to_string());
        assert_eq!(compile_expr(&e, reactive()), "speed");
    }

    #[test]
    fn binop_add() {
        let e = Expr::BinOp(
            Box::new(Expr::Literal(Literal::Int(1))),
            BinOpKind::Add,
            Box::new(Expr::Literal(Literal::Int(2))),
        );
        assert_eq!(compile_expr(&e, build_fn()), "(1 + 2)");
    }

    #[test]
    fn binop_selfprop_plus_int() {
        let e = Expr::BinOp(
            Box::new(Expr::SelfProp("count".to_string())),
            BinOpKind::Add,
            Box::new(Expr::Literal(Literal::Int(1))),
        );
        // In build() context, SelfProp dereferences.
        assert_eq!(compile_expr(&e, build_fn()), "(*count.get() + 1)");
    }

    #[test]
    fn interpolated_single_var() {
        let e = Expr::Interpolated(vec![
            InterpPart::Lit("Count: ".to_string()),
            InterpPart::Expr(Box::new(Expr::Ident("count".to_string()))),
        ]);
        let out = compile_expr(&e, build_fn());
        assert!(out.contains("alloc::format!"), "should use alloc::format!: {}", out);
        assert!(out.contains("Count: {}"), "format string should contain 'Count: {{}}': {}", out);
        assert!(out.contains("count"), "format args should include 'count': {}", out);
    }

    #[test]
    fn raw_expr_passthrough() {
        let e = Expr::Raw(RawExpr {
            text: "*foo.get()".to_string(),
            span: Span::default(),
        });
        assert_eq!(compile_expr(&e, build_fn()), "*foo.get()");
    }

    #[test]
    fn unary_not() {
        let e = Expr::Unary(UnaryOp::Not, Box::new(Expr::Literal(Literal::Bool(true))));
        assert_eq!(compile_expr(&e, build_fn()), "(!true)");
    }
}
