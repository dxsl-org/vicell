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
