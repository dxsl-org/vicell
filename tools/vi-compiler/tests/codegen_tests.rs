use vi_compiler::{codegen::CodeGen, compile_str};

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
